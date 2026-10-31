import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { open } from "@tauri-apps/plugin-dialog";
import { api, AppAction, AUDIO_EXTENSIONS, ModelCatalog, modelsReady, ProgressEvent, Recording } from "./api";
import { AskPanel } from "./components/AskPanel";
import { QuickOpen } from "./components/QuickOpen";
import { ShortcutHelp } from "./components/ShortcutHelp";
import { AppCommand, Command, IS_MAC, matchShortcut, sendCommand } from "./shortcuts";
import { tagCounts } from "./tags";
import { ModelSetup } from "./components/ModelSetup";
import { Recorder } from "./components/Recorder";
import { Sidebar } from "./components/Sidebar";
import { RecordingView } from "./components/RecordingView";
import { SettingsDialog } from "./components/SettingsDialog";
import "./App.css";

export default function App() {
  const [recordings, setRecordings] = useState<Recording[]>([]);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [progress, setProgress] = useState<Record<string, ProgressEvent>>({});
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [dragging, setDragging] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [catalog, setCatalog] = useState<ModelCatalog | null>(null);
  const [setupLater, setSetupLater] = useState(false);
  /** null이면 빠른 찾기 창을 닫은 상태. 문자열은 처음 넣을 검색어 */
  const [quickOpen, setQuickOpenState] = useState<{ query: string; nonce: number } | null>(null);
  // 이미 열려 있어도 새 검색어로 다시 열리도록 매번 새 값을 만든다
  const setQuickOpen = useCallback(
    (query: string | null) => setQuickOpenState(query === null ? null : { query, nonce: Date.now() }),
    [],
  );
  const [helpOpen, setHelpOpen] = useState(false);
  /** ⌘(윈도우는 Ctrl)만 잠깐 누르고 있으면 단축키 미리보기를 띄운다 */
  const [peekOpen, setPeekOpen] = useState(false);
  const [askOpen, setAskOpen] = useState(false);
  /** 링크나 검색 결과로 녹음을 열 때 옮겨 갈 재생 위치. nonce로 같은 시각을 다시 골라도 반영한다 */
  const [seekTo, setSeekTo] = useState<{ id: string; time: number; nonce: number } | null>(null);

  const refresh = useCallback(async () => {
    setRecordings(await api.listRecordings());
  }, []);
  /** 동기화로 새로 온 녹음을 가려내려고 지금까지 본 녹음 id를 들고 있는다 */
  const knownIds = useRef<string[]>([]);
  useEffect(() => {
    knownIds.current = recordings.map((r) => r.id);
  }, [recordings]);

  const importPaths = useCallback(
    async (paths: string[]): Promise<Recording[]> => {
      const audio = paths.filter((p) => AUDIO_EXTENSIONS.includes(p.split(".").pop()?.toLowerCase() ?? ""));
      if (audio.length === 0) {
        setError("가져올 수 있는 오디오 파일이 없습니다");
        return [];
      }
      try {
        const imported = await api.importRecordings(audio);
        await refresh();
        setSelectedId(imported[0].id);
        return imported;
      } catch (e) {
        setError(String(e));
        return [];
      }
    },
    [refresh],
  );

  /**
   * 녹음을 기본 설정(언어는 설정값, 화자 수는 자동)으로 처리한다. 백엔드가 하나씩 차례로 처리한다.
   * 자동 요약이 켜져 있으면 처리가 끝난 뒤 기본 템플릿으로 요약한다
   */
  const processDefaults = useCallback(
    async (recordings: Recording[]) => {
      const settings = await api.getSettings();
      for (const r of recordings) {
        api
          .processRecording(r.id, null, settings.defaultLanguages)
          .then(() => (settings.autoSummarize ? api.summarize(r.id, null, null) : undefined))
          // 요약이 있으면 요약을 보고 분류한다. 분류는 덤이라 실패해도 알리지 않고 기록만 남긴다
          .then(() => (settings.autoTag ? api.autoTag(r.id).catch((e) => console.warn(`자동 분류 실패 ${r.title}: ${e}`)) : undefined))
          .catch((e) => setError(`${r.title}: ${e}`))
          .finally(refresh);
      }
    },
    [refresh],
  );

  /** 설정에서 자동 처리를 켰으면 처리한다 */
  const autoProcess = useCallback(
    async (recordings: Recording[]) => {
      if (recordings.length > 0 && (await api.getSettings()).autoProcess) await processDefaults(recordings);
    },
    [processDefaults],
  );

  const pickFiles = useCallback(async () => {
    const picked = await open({
      multiple: true,
      filters: [{ name: "오디오", extensions: AUDIO_EXTENSIONS }],
    });
    if (picked) await autoProcess(await importPaths(picked));
  }, [importPaths, autoProcess]);

  useEffect(() => {
    api.modelCatalog().then(setCatalog);
  }, []);

  const clearSeek = useCallback(() => setSeekTo(null), []);

  const openRecording = useCallback(
    async (id: string, time: number | null) => {
      await refresh();
      setSelectedId(id);
      setSeekTo(time == null ? null : { id, time, nonce: Date.now() });
    },
    [refresh],
  );

  const handleAction = useCallback(
    async (a: AppAction) => {
      switch (a.type) {
        case "toggleRecording":
          return sendCommand("toggle-recording");
        case "startRecording":
          return sendCommand("start-recording");
        case "stopRecording":
          return sendCommand("stop-recording");
        case "showWindow":
          return;
        case "open":
          return openRecording(a.id, a.time);
        case "import": {
          const imported = await importPaths(a.paths);
          if (a.process) await processDefaults(imported);
          else await autoProcess(imported);
          return;
        }
        case "bookmark":
          return api
            .addNote(null, null, "")
            .then(refresh)
            .catch((e) => setError(String(e)));
        case "search":
          return setQuickOpen(a.query);
        case "blocked":
          return setError(a.message);
      }
    },
    [importPaths, openRecording, processDefaults, autoProcess, refresh, setQuickOpen],
  );

  // 전역 단축키와 sori:// 링크. 구독을 마친 뒤 그 전에 들어온 요청(링크로 앱을 띄운 경우)을 받아 처리한다
  const actionRef = useRef(handleAction);
  actionRef.current = handleAction;
  useEffect(() => {
    let cancelled = false;
    const unlisten = api.onAction((a) => actionRef.current(a));
    unlisten.then(async () => {
      if (cancelled) return;
      const ready = await api.frontendReady();
      if (ready.shortcutError) setError(ready.shortcutError);
      for (const a of ready.pending) await actionRef.current(a);
    });
    return () => {
      cancelled = true;
      unlisten.then((f) => f());
    };
  }, []);

  const runCommand = (command: Command | AppCommand) => {
    switch (command) {
      case "import":
        return pickFiles();
      case "settings":
        return setSettingsOpen(true);
      case "search":
        return setQuickOpen("");
      case "help":
        return setHelpOpen(true);
      case "ask-all":
        return setAskOpen(true);
      case "prev":
      case "next": {
        if (recordings.length === 0) return;
        const index = recordings.findIndex((r) => r.id === selectedId);
        const next = command === "next" ? Math.min(index + 1, recordings.length - 1) : Math.max(index - 1, 0);
        return setSelectedId(recordings[index < 0 ? 0 : next].id);
      }
      default:
        return sendCommand(command);
    }
  };

  const modalOpen = settingsOpen || helpOpen || askOpen || quickOpen !== null;
  const commandRef = useRef(runCommand);
  commandRef.current = runCommand;
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      // 한글 조합 중 Enter, Space는 글자를 완성하는 키다
      if (e.isComposing || e.keyCode === 229) return;
      // 빠른 찾기 같은 창이 이미 처리한 키다. 창이 닫히며 같은 키를 또 받는 경우를 막는다
      if (e.defaultPrevented) return;
      const shortcut = matchShortcut(e);
      if (!shortcut || modalOpen) return;
      // 재생기에 포커스가 있으면 재생기가 Space와 화살표를 직접 처리한다. 두 번 처리하면 재생이 바로 멈춘다
      if (e.target instanceof HTMLMediaElement && [" ", "ArrowLeft", "ArrowRight"].includes(shortcut.key)) return;
      // Space를 누르고 있으면 재생과 멈춤이 빠르게 반복된다
      if (e.repeat && shortcut.key === " ") return;
      e.preventDefault();
      commandRef.current(shortcut.run);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [modalOpen]);

  // ⌘ 하나만 0.6초 누르고 있으면 미리보기를 띄우고, 떼거나 다른 키를 함께 누르면 닫는다.
  // ⌘C처럼 바로 다른 키를 누르는 평소 사용에서는 뜨지 않는다. ⌘Tab으로 다른 앱에 가면 keyup이 오지 않아 blur에서도 닫는다
  useEffect(() => {
    const modKey = IS_MAC ? "Meta" : "Control";
    let timer: number | undefined;
    const close = () => {
      window.clearTimeout(timer);
      timer = undefined;
      setPeekOpen(false);
    };
    const onDown = (e: KeyboardEvent) => {
      if (e.key === modKey && !e.repeat && !e.shiftKey && !e.altKey) {
        close();
        timer = window.setTimeout(() => setPeekOpen(true), 600);
      } else if (e.key !== modKey) {
        close();
      }
    };
    const onUp = (e: KeyboardEvent) => e.key === modKey && close();
    window.addEventListener("keydown", onDown, true);
    window.addEventListener("keyup", onUp, true);
    window.addEventListener("blur", close);
    window.addEventListener("mousedown", close, true);
    return () => {
      close();
      window.removeEventListener("keydown", onDown, true);
      window.removeEventListener("keyup", onUp, true);
      window.removeEventListener("blur", close);
      window.removeEventListener("mousedown", close, true);
    };
  }, []);

  useEffect(() => {
    refresh();
    const unlistenProgress = api.onProgress((e) => {
      setProgress((prev) => ({ ...prev, [e.id]: e }));
      // 첫 이벤트는 백엔드가 상태를 "처리 중"으로 저장한 뒤에 오므로 이때 목록을 다시 읽는다
      if ((e.stage === "decoding" || e.stage === "waiting") && e.progress === 0) refresh();
    });
    // 다른 기기에서 녹음이 오거나 지워지면 목록을 다시 읽는다. 이번에 새로 온 녹음(iPhone 녹음 등)은
    // 자동 처리가 켜져 있으면 받아 적고 요약, 분류까지 한다. 예전부터 대기 중이던 녹음은 건드리지 않는다
    const unlistenSynced = api.onRecordingsSynced(async () => {
      const before = new Set(knownIds.current);
      const list = await api.listRecordings();
      setRecordings(list);
      if (before.size === 0) return;
      const arrived = list.filter((r) => !before.has(r.id) && r.status === "new" && r.duration);
      if (arrived.length) await autoProcess(arrived);
    });
    const unlistenDrop = getCurrentWebview().onDragDropEvent((e) => {
      if (e.payload.type === "enter" || e.payload.type === "over") setDragging(true);
      else setDragging(false);
      if (e.payload.type === "drop") importPaths(e.payload.paths).then(autoProcess);
    });
    return () => {
      unlistenProgress.then((f) => f());
      unlistenSynced.then((f) => f());
      unlistenDrop.then((f) => f());
    };
  }, [refresh, importPaths, autoProcess]);

  const selected = recordings.find((r) => r.id === selectedId) ?? null;
  const allTags = useMemo(() => tagCounts(recordings), [recordings]);

  return (
    <div className="app">
      <Sidebar
        recordings={recordings}
        selectedId={selectedId}
        progress={progress}
        onSelect={setSelectedId}
        onImport={pickFiles}
        onSearch={() => setQuickOpen("")}
        onOpenSettings={() => setSettingsOpen(true)}
        recorder={
          <Recorder
            onStarted={async (r) => {
              await refresh();
              setSelectedId(r.id);
            }}
            onStopped={async (r) => {
              await refresh();
              setSelectedId(r.id);
              if (r.duration) await autoProcess([r]);
            }}
            onError={setError}
          />
        }
      />
      <main className="main">
        {catalog && !modelsReady(catalog) && setupLater && (
          <div className="setup-banner" role="status">
            모델이 없어 받아 적기를 할 수 없습니다.
            <button className="primary" onClick={() => setSetupLater(false)}>
              모델 설치
            </button>
          </div>
        )}
        {catalog && !modelsReady(catalog) && !setupLater ? (
          <ModelSetup catalog={catalog} onInstalled={setCatalog} onLater={() => setSetupLater(true)} />
        ) : selected ? (
          <RecordingView
            // 상태(녹음 중, 처리 중, 완료 등)가 바뀌면 화면을 새로 만들어 최신 결과를 읽는다.
            // 처리 중에 다른 녹음을 봤다 돌아와도 끝난 결과가 보인다
            key={`${selected.id}-${selected.status}`}
            recording={selected}
            allTags={allTags}
            progress={progress[selected.id]}
            seekTo={seekTo?.id === selected.id ? seekTo : null}
            onSeekDone={clearSeek}
            onChanged={refresh}
            onDeleted={async () => {
              setSelectedId(null);
              await refresh();
            }}
          />
        ) : (
          <div className="empty">
            <p className="empty-title">녹음 파일을 끌어다 놓거나 가져오세요</p>
            <p className="muted">m4a, mp3, wav, flac, ogg, mp4 등. 받아 적기와 화자 분리는 이 컴퓨터 안에서만 이뤄집니다.</p>
            <button className="primary" onClick={pickFiles}>
              파일 가져오기
            </button>
          </div>
        )}
      </main>
      {dragging && <div className="drop-overlay">여기에 놓으면 가져옵니다</div>}
      {error && (
        <div className="toast" role="alert" onClick={() => setError(null)}>
          {error}
        </div>
      )}
      {quickOpen !== null && (
        <QuickOpen
          key={quickOpen.nonce}
          initialQuery={quickOpen.query}
          onClose={() => setQuickOpen(null)}
          onOpen={(id, time) => {
            setQuickOpen(null);
            openRecording(id, time);
          }}
        />
      )}
      {helpOpen && <ShortcutHelp onClose={() => setHelpOpen(false)} />}
      {peekOpen && !helpOpen && !modalOpen && <ShortcutHelp peek onClose={() => setPeekOpen(false)} />}
      {askOpen && (
        <div className="modal-backdrop" onClick={() => setAskOpen(false)} onKeyDown={(e) => e.key === "Escape" && setAskOpen(false)}>
          <div className="modal ask-modal" role="dialog" aria-label="모든 녹음에 물어보기" onClick={(e) => e.stopPropagation()}>
            <div className="ask-modal-head">
              <h2>모든 녹음에 물어보기</h2>
              <button className="ghost" onClick={() => setAskOpen(false)}>
                닫기
              </button>
            </div>
            <AskPanel
              recordingId={null}
              onCite={(id, time) => {
                if (!id) return;
                setAskOpen(false);
                openRecording(id, time);
              }}
            />
          </div>
        </div>
      )}
      {settingsOpen && (
        <SettingsDialog
          onClose={() => {
            setSettingsOpen(false);
            // 설정에서 모델을 받았을 수 있으니 다시 확인한다
            api.modelCatalog().then(setCatalog);
          }}
        />
      )}
    </div>
  );
}

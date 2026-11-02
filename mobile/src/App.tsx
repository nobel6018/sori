import { ReactNode, RefObject, useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { tagCounts } from "../../src/tags";
import { checkForUpdate, UpdateInfo } from "../../src/updateCheck";
import { api, RecorderState, RecordingItem, SyncStatus } from "./api";
import { Detail } from "./Detail";
import { List } from "./List";
import { Recorder } from "./Recorder";
import { Settings } from "./Settings";
import { useEdgeSwipeBack } from "./useEdgeSwipeBack";

type Screen = { name: "list" } | { name: "detail"; id: string } | { name: "settings" };

export function App() {
  const [screen, setScreen] = useState<Screen>({ name: "list" });
  const [recordings, setRecordings] = useState<RecordingItem[]>([]);
  const [sync, setSync] = useState<SyncStatus | null>(null);
  /** 녹음 중이면 녹음 화면을 다른 화면 위에 띄운다 */
  const [recorder, setRecorder] = useState<RecorderState | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [update, setUpdate] = useState<UpdateInfo | null>(null);
  const allTags = useMemo(() => tagCounts(recordings), [recordings]);
  /** iPhone에서 미리 전사하고 있는 녹음 */
  const [previewing, setPreviewing] = useState<string | null>(null);
  /** 미리 전사가 실패한 녹음. 이번 실행에서는 다시 하지 않는다 */
  const previewFailed = useRef<Set<string>>(new Set());

  // 켤 때 한 번 새 버전을 확인한다. 업데이트는 AltStore가 하므로 알리기만 한다(docs/mobile.md)
  useEffect(() => {
    checkForUpdate(/^sori-ios\.ipa$/).then(setUpdate);
  }, []);

  const refresh = useCallback(() => {
    api.listRecordings().then(setRecordings).catch((e) => setError(String(e)));
  }, []);

  useEffect(() => {
    refresh();
    api.syncStatus().then(setSync);
    // 앱을 다시 켰거나 동작 버튼, 단축어로 녹음이 바뀌었으면 녹음 화면을 그 상태에 맞춘다
    const syncRecorder = () => api.recorderState().then((s) => setRecorder(s.id ? s : null));
    syncRecorder();
    const subs = [
      api.onSyncStatus(setSync),
      api.onRecordingsSynced(refresh),
      api.onRecordingChanged(() => refresh()),
      api.onRecorderChanged(() => {
        syncRecorder();
        refresh();
      }),
    ];
    // 앱으로 돌아오면 다른 기기의 변경(Mac에서 끝난 전사 등)을 바로 가져온다
    const onVisible = () => {
      if (document.visibilityState !== "visible") return;
      syncRecorder();
      api.syncNow();
    };
    document.addEventListener("visibilitychange", onVisible);
    return () => {
      subs.forEach((s) => s.then((f) => f()));
      document.removeEventListener("visibilitychange", onVisible);
    };
  }, [refresh]);

  // 앱이 화면에 있는 동안 전사가 없는 녹음을 하나씩 iPhone에서 미리 전사한다. 긴 전사는 백그라운드에서 끊길 수 있다
  useEffect(() => {
    if (previewing || recorder?.id || document.visibilityState !== "visible") return;
    const next = recordings.find(
      (r) => r.audioAvailable && !r.hasTranscript && !r.hasPreview && r.status !== "recording" && !previewFailed.current.has(r.id),
    );
    if (!next) return;
    let cancelled = false;
    api.getPreferences().then((prefs) => {
      if (cancelled || !prefs.previewTranscribe) return;
      setPreviewing(next.id);
      api
        .previewTranscribe(next.id)
        .catch((e) => {
          previewFailed.current.add(next.id);
          console.warn(`미리 전사 실패 ${next.title}: ${e}`);
        })
        .finally(() => {
          setPreviewing(null);
          refresh();
        });
    });
    return () => {
      cancelled = true;
    };
  }, [recordings, previewing, recorder, refresh]);

  /**
   * 목록은 다른 화면이 떠 있어도 내리지 않고 아래 층(.list-layer.under)에 고정해 둔다. 가장자리를 밀 때 뒤에 보이고,
   * 돌아오면 보던 위치 그대로다. 문서 스크롤은 늘 맨 위 화면 몫이라 상태 표시줄을 누르면 그 화면이 맨 위로 간다
   */
  const listLayer = useRef<HTMLDivElement>(null);
  const listScroll = useRef(0);
  const open = useCallback((next: Screen) => {
    setScreen((current) => {
      // 목록을 고정하면 문서 높이가 줄어 scrollY가 깎이므로 바꾸기 전에 읽어 둔다
      if (current.name === "list") listScroll.current = window.scrollY;
      return next;
    });
  }, []);
  useLayoutEffect(() => {
    if (screen.name === "list") {
      window.scrollTo(0, listScroll.current);
    } else {
      if (listLayer.current) listLayer.current.scrollTop = listScroll.current;
      window.scrollTo(0, 0);
    }
  }, [screen.name]);

  const backToList = useCallback(() => {
    setScreen({ name: "list" });
    refresh();
  }, [refresh]);

  const startRecording = async () => {
    setError(null);
    try {
      const meta = await api.startRecording();
      setRecorder({ id: meta.id, recording: true, time: 0, level: 0 });
      refresh();
    } catch (e) {
      setError(String(e));
    }
  };

  return (
    <div className="app">
      {error && (
        <div className="toast error" role="alert" onClick={() => setError(null)}>
          {error}
        </div>
      )}
      {update && screen.name === "list" && (
        <div className="update-banner" role="status">
          <span>
            새 버전 {update.latest}이 있습니다(지금 {update.current}). AltStore의 My Apps에서 업데이트하세요.
          </span>
          <button className="text-button" onClick={() => setUpdate(null)}>
            닫기
          </button>
        </div>
      )}
      <div ref={listLayer} className={`list-layer ${screen.name === "list" ? "" : "under"}`} inert={screen.name !== "list"}>
        <List
          visible={screen.name === "list"}
          recordings={recordings}
          previewing={previewing}
          sync={sync}
          onOpen={(id) => open({ name: "detail", id })}
          onSettings={() => open({ name: "settings" })}
          onRecord={startRecording}
          onRefresh={() => {
            refresh();
            api.syncNow();
          }}
        />
      </div>
      {screen.name === "detail" && (
        <SwipeBack key={screen.id} under={listLayer} onBack={backToList} enabled={!recorder?.id}>
          <Detail id={screen.id} allTags={allTags} onBack={backToList} />
        </SwipeBack>
      )}
      {screen.name === "settings" && (
        <SwipeBack under={listLayer} onBack={backToList} enabled={!recorder?.id}>
          <Settings sync={sync} onBack={backToList} />
        </SwipeBack>
      )}
      {recorder?.id && (
        <Recorder
          id={recorder.id}
          onStopped={(id) => {
            setRecorder(null);
            refresh();
            open({ name: "detail", id });
          }}
          onError={setError}
        />
      )}
    </div>
  );
}

/** 목록 위에 쌓이는 화면. 왼쪽 가장자리에서 밀면 아래 목록이 드러나며 돌아간다. 녹음 화면이 떠 있으면 끈다 */
function SwipeBack({
  under,
  onBack,
  enabled,
  children,
}: {
  under: RefObject<HTMLDivElement | null>;
  onBack: () => void;
  enabled: boolean;
  children: ReactNode;
}) {
  const ref = useRef<HTMLDivElement>(null);
  useEdgeSwipeBack(ref, under, onBack, enabled);
  return (
    <div ref={ref} className="swipe-back">
      {children}
    </div>
  );
}

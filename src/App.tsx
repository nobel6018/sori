import { useCallback, useEffect, useState } from "react";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { open } from "@tauri-apps/plugin-dialog";
import { api, AUDIO_EXTENSIONS, ProgressEvent, Recording } from "./api";
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

  const refresh = useCallback(async () => {
    setRecordings(await api.listRecordings());
  }, []);

  const importPaths = useCallback(
    async (paths: string[]) => {
      const audio = paths.filter((p) => AUDIO_EXTENSIONS.includes(p.split(".").pop()?.toLowerCase() ?? ""));
      if (audio.length === 0) {
        setError("가져올 수 있는 오디오 파일이 없습니다");
        return;
      }
      try {
        const imported = await api.importRecordings(audio);
        await refresh();
        setSelectedId(imported[0].id);
      } catch (e) {
        setError(String(e));
      }
    },
    [refresh],
  );

  const pickFiles = useCallback(async () => {
    const picked = await open({
      multiple: true,
      filters: [{ name: "오디오", extensions: AUDIO_EXTENSIONS }],
    });
    if (picked) await importPaths(picked);
  }, [importPaths]);

  useEffect(() => {
    refresh();
    const unlistenProgress = api.onProgress((e) => {
      setProgress((prev) => ({ ...prev, [e.id]: e }));
      // 첫 이벤트는 백엔드가 상태를 "처리 중"으로 저장한 뒤에 오므로 이때 목록을 다시 읽는다
      if (e.stage === "decoding" && e.progress === 0) refresh();
    });
    const unlistenDrop = getCurrentWebview().onDragDropEvent((e) => {
      if (e.payload.type === "enter" || e.payload.type === "over") setDragging(true);
      else setDragging(false);
      if (e.payload.type === "drop") importPaths(e.payload.paths);
    });
    return () => {
      unlistenProgress.then((f) => f());
      unlistenDrop.then((f) => f());
    };
  }, [refresh, importPaths]);

  const selected = recordings.find((r) => r.id === selectedId) ?? null;

  return (
    <div className="app">
      <Sidebar
        recordings={recordings}
        selectedId={selectedId}
        progress={progress}
        onSelect={setSelectedId}
        onImport={pickFiles}
        onOpenSettings={() => setSettingsOpen(true)}
      />
      <main className="main">
        {selected ? (
          <RecordingView
            key={selected.id}
            recording={selected}
            progress={progress[selected.id]}
            onChanged={refresh}
            onDeleted={async () => {
              setSelectedId(null);
              await refresh();
            }}
          />
        ) : (
          <div className="empty">
            <p className="empty-title">녹음 파일을 끌어다 놓거나 가져오세요</p>
            <p className="muted">m4a, mp3, wav, flac, ogg, mp4 등. 모든 처리는 이 컴퓨터 안에서만 이뤄집니다.</p>
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
      {settingsOpen && <SettingsDialog onClose={() => setSettingsOpen(false)} />}
    </div>
  );
}

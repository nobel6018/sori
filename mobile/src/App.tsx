import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { tagCounts } from "../../src/tags";
import { api, RecorderState, RecordingItem, SyncStatus } from "./api";
import { Detail } from "./Detail";
import { List } from "./List";
import { Recorder } from "./Recorder";
import { Settings } from "./Settings";

type Screen = { name: "list" } | { name: "detail"; id: string } | { name: "settings" };

export function App() {
  const [screen, setScreen] = useState<Screen>({ name: "list" });
  const [recordings, setRecordings] = useState<RecordingItem[]>([]);
  const [sync, setSync] = useState<SyncStatus | null>(null);
  /** 녹음 중이면 녹음 화면을 다른 화면 위에 띄운다 */
  const [recorder, setRecorder] = useState<RecorderState | null>(null);
  const [error, setError] = useState<string | null>(null);
  const allTags = useMemo(() => tagCounts(recordings), [recordings]);
  /** iPhone에서 미리 전사하고 있는 녹음 */
  const [previewing, setPreviewing] = useState<string | null>(null);
  /** 미리 전사가 실패한 녹음. 이번 실행에서는 다시 하지 않는다 */
  const previewFailed = useRef<Set<string>>(new Set());

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
      {screen.name === "list" && (
        <List
          recordings={recordings}
          previewing={previewing}
          sync={sync}
          onOpen={(id) => setScreen({ name: "detail", id })}
          onSettings={() => setScreen({ name: "settings" })}
          onRecord={startRecording}
          onRefresh={() => {
            refresh();
            api.syncNow();
          }}
        />
      )}
      {screen.name === "detail" && (
        <Detail
          key={screen.id}
          id={screen.id}
          allTags={allTags}
          onBack={() => {
            setScreen({ name: "list" });
            refresh();
          }}
        />
      )}
      {screen.name === "settings" && <Settings sync={sync} onBack={() => setScreen({ name: "list" })} />}
      {recorder?.id && (
        <Recorder
          id={recorder.id}
          onStopped={(id) => {
            setRecorder(null);
            refresh();
            setScreen({ name: "detail", id });
          }}
          onError={setError}
        />
      )}
    </div>
  );
}

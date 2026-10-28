import { useEffect, useState } from "react";
import { api, formatTime, Recording } from "../api";

interface Props {
  onStarted: (recording: Recording) => void;
  onStopped: (recording: Recording) => void;
  onError: (message: string) => void;
}

export function Recorder({ onStarted, onStopped, onError }: Props) {
  const [active, setActive] = useState<Recording | null>(null);
  const [startedAt, setStartedAt] = useState(0);
  const [elapsed, setElapsed] = useState(0);
  const [levels, setLevels] = useState<number[]>([]);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    const unlisten = api.onLevel(setLevels);
    return () => {
      unlisten.then((f) => f());
    };
  }, []);

  // 장치가 빠지는 등으로 녹음이 끊기면 백엔드가 알려 준다. 멈추기와 같은 마무리를 한다
  useEffect(() => {
    if (!active) return;
    const unlisten = api.onRecordingInterrupted(() => stop());
    return () => {
      unlisten.then((f) => f());
    };
    // stop은 active가 바뀔 때마다 새로 만들어지므로 active 기준으로만 다시 구독한다
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [active]);

  useEffect(() => {
    if (!active) return;
    const timer = setInterval(() => setElapsed((Date.now() - startedAt) / 1000), 250);
    return () => clearInterval(timer);
  }, [active, startedAt]);

  const start = async () => {
    setBusy(true);
    try {
      const recording = await api.startRecording();
      setActive(recording);
      setStartedAt(Date.now());
      setElapsed(0);
      onStarted(recording);
    } catch (e) {
      onError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const stop = async () => {
    setBusy(true);
    try {
      const recording = await api.stopRecording();
      setActive(null);
      setLevels([]);
      onStopped(recording);
    } catch (e) {
      onError(String(e));
    } finally {
      setBusy(false);
    }
  };

  if (!active) {
    return (
      <div className="recorder">
        <button className="record-button" onClick={start} disabled={busy}>
          <span className="record-dot" aria-hidden />
          녹음 시작
        </button>
      </div>
    );
  }

  return (
    <div className="recorder recording" aria-live="polite">
      <div className="recorder-row">
        <span className="record-dot live" aria-hidden />
        <span className="recorder-time">{formatTime(elapsed)}</span>
        <button className="stop-button" onClick={stop} disabled={busy}>
          멈추기
        </button>
      </div>
      {active.tracks.map((track, i) => {
        const label = track.startsWith("system") ? "시스템" : "마이크";
        const level = levels[i] ?? 0;
        return (
          <div key={track} className="level-row">
            <span className="level-label">{label}</span>
            <div
              className="level-track"
              role="meter"
              aria-label={`${label} 음량`}
              aria-valuemin={0}
              aria-valuemax={1}
              aria-valuenow={level}
            >
              <div className="level-bar" style={{ width: `${level * 100}%` }} />
            </div>
          </div>
        );
      })}
    </div>
  );
}

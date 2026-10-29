import { useEffect, useRef, useState } from "react";
import { formatTime } from "../../src/api";
import { api, Note } from "./api";

/** 녹음 중 화면. 다른 화면 위에 덮이고, 멈추면 그 녹음의 상세 화면으로 간다 */
export function Recorder({ id, onStopped, onError }: { id: string; onStopped: (id: string) => void; onError: (e: string) => void }) {
  const [time, setTime] = useState(0);
  const [level, setLevel] = useState(0);
  const [notes, setNotes] = useState<Note[]>([]);
  const [text, setText] = useState("");
  const [stopping, setStopping] = useState(false);
  const input = useRef<HTMLInputElement>(null);

  useEffect(() => {
    const timer = window.setInterval(() => {
      api.recorderState().then((s) => {
        setTime(s.time);
        setLevel(s.level);
      });
    }, 250);
    return () => window.clearInterval(timer);
  }, []);

  const addNote = async (body: string) => {
    try {
      const meta = await api.addNote(id, null, body);
      setNotes(meta.notes ?? []);
      setText("");
    } catch (e) {
      onError(String(e));
    }
  };

  const stop = async () => {
    setStopping(true);
    try {
      await api.stopRecording();
      onStopped(id);
    } catch (e) {
      onError(String(e));
      setStopping(false);
    }
  };

  return (
    <div className="recorder" role="dialog" aria-label="녹음 중">
      <div className="recorder-top">
        <span className="rec-label">
          <span className="record-dot small" /> 녹음 중
        </span>
        <div className="timer">{formatTime(time)}</div>
        <div className="meter" aria-hidden>
          <div className="meter-fill" style={{ transform: `scaleX(${Math.max(0.02, level)})` }} />
        </div>
        <p className="muted small">화면을 끄거나 다른 앱으로 가도 녹음은 이어집니다.</p>
      </div>

      <ul className="recorder-notes">
        {notes.map((n) => (
          <li key={n.id ?? `${n.time}-${n.text}`}>
            <span className="note-time">{formatTime(n.time)}</span>
            <span>{n.text || "북마크"}</span>
          </li>
        ))}
      </ul>

      <form
        className="note-form"
        onSubmit={(e) => {
          e.preventDefault();
          if (text.trim()) addNote(text);
        }}
      >
        <input ref={input} value={text} onChange={(e) => setText(e.target.value)} placeholder="지금 시각에 메모" enterKeyHint="done" />
        {text.trim() ? (
          <button type="submit">메모</button>
        ) : (
          <button type="button" onClick={() => addNote("")}>
            북마크
          </button>
        )}
      </form>

      <button className="stop-button" onClick={stop} disabled={stopping} aria-label="녹음 멈추기">
        <span className="stop-square" />
      </button>
    </div>
  );
}

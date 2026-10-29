import { useState } from "react";
import { api, formatTime, Recording } from "../api";
import { IS_MAC } from "../shortcuts";

interface Props {
  recording: Recording;
  /** 녹음 중이면 시각을 백엔드가 녹음 시작부터 잰다. 아니면 지금 재생 위치에 단다 */
  recordingNow: boolean;
  currentTime: number;
  onSeek: (time: number) => void;
  onChanged: () => void;
}

/**
 * 녹음하며 남기는 메모와 북마크. 요약할 때 함께 넘겨, 표시한 곳을 더 중요하게 다루게 한다
 */
export function NotesPanel({ recording, recordingNow, currentTime, onSeek, onChanged }: Props) {
  const [text, setText] = useState("");
  const [error, setError] = useState<string | null>(null);

  const add = async (body: string) => {
    setError(null);
    try {
      await api.addNote(recording.id, recordingNow ? null : currentTime, body);
      setText("");
      onChanged();
    } catch (e) {
      setError(String(e));
    }
  };

  const remove = async (index: number) => {
    await api.deleteNote(recording.id, index);
    onChanged();
  };

  const bookmarkKey = IS_MAC ? "⌘B" : "Ctrl+B";
  return (
    <section className="notes">
      <form
        className="note-form"
        onSubmit={(e) => {
          e.preventDefault();
          if (text.trim()) add(text);
        }}
      >
        <input
          value={text}
          onChange={(e) => setText(e.target.value)}
          placeholder={recordingNow ? "메모를 쓰고 Enter. 지금 시각이 함께 저장됩니다" : `메모를 쓰고 Enter. 재생 위치 ${formatTime(currentTime)}에 저장됩니다`}
          aria-label="메모"
        />
        <button type="button" onClick={() => add("")} title={`북마크 (${bookmarkKey})`}>
          북마크
        </button>
      </form>
      {error && <div className="error">{error}</div>}
      {recording.notes.length === 0 ? (
        <p className="muted hint">
          중요한 순간에 북마크({bookmarkKey}, sori가 뒤에 있으면 {IS_MAC ? "⌥⇧⌘B" : "Ctrl+Alt+Shift+B"})를 누르거나 짧게 메모하세요. 요약할 때 이 부분을 더 중요하게 다룹니다.
        </p>
      ) : (
        <ul className="note-list">
          {recording.notes.map((n, i) => (
            <li key={`${n.time}-${i}`}>
              <button className="time" onClick={() => onSeek(n.time)} disabled={recordingNow}>
                {formatTime(n.time)}
              </button>
              <span className={n.text ? "" : "muted"}>{n.text || "북마크"}</span>
              <button className="ghost note-delete" onClick={() => remove(i)} aria-label="메모 지우기">
                지우기
              </button>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}

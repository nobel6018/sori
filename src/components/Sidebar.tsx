import { formatTime, ProgressEvent, Recording, Status } from "../api";

const STATUS_LABEL: Record<Status, string> = {
  new: "대기",
  processing: "처리 중",
  done: "완료",
  failed: "실패",
};

interface Props {
  recordings: Recording[];
  selectedId: string | null;
  progress: Record<string, ProgressEvent>;
  onSelect: (id: string) => void;
  onImport: () => void;
  onOpenSettings: () => void;
}

export function Sidebar({ recordings, selectedId, progress, onSelect, onImport, onOpenSettings }: Props) {
  return (
    <aside className="sidebar">
      <header className="sidebar-header">
        <span className="brand">sori</span>
        <button onClick={onImport}>가져오기</button>
      </header>
      <ul className="recording-list">
        {recordings.map((r) => {
          const p = progress[r.id];
          return (
            <li key={r.id}>
              <button className={`recording-item ${r.id === selectedId ? "selected" : ""}`} onClick={() => onSelect(r.id)}>
                <span className="recording-title">{r.title}</span>
                <span className="recording-meta">
                  <span className={`badge badge-${r.status}`}>
                    {r.status === "processing" && p ? `${Math.round(p.progress * 100)}%` : STATUS_LABEL[r.status]}
                  </span>
                  {r.duration != null && <span>{formatTime(r.duration)}</span>}
                  <span>{new Date(r.createdAt).toLocaleDateString("ko-KR")}</span>
                </span>
              </button>
            </li>
          );
        })}
      </ul>
      <footer className="sidebar-footer">
        <button className="ghost" onClick={onOpenSettings}>
          설정
        </button>
      </footer>
    </aside>
  );
}

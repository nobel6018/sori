import { formatTime } from "../../src/api";
import { RecordingItem, SyncStatus, syncLabel } from "./api";

export function List({
  recordings,
  sync,
  onOpen,
  onSettings,
  onRecord,
  onRefresh,
}: {
  recordings: RecordingItem[];
  sync: SyncStatus | null;
  onOpen: (id: string) => void;
  onSettings: () => void;
  onRecord: () => void;
  onRefresh: () => void;
}) {
  return (
    <div className="screen">
      <header className="bar">
        <h1>sori</h1>
        <div className="bar-actions">
          {sync && sync.provider !== "local" && (
            <button className={`pill ${sync.error ? "error" : ""}`} onClick={onRefresh} disabled={sync.running}>
              {syncLabel(sync)}
            </button>
          )}
          <button className="text-button" onClick={onSettings}>
            설정
          </button>
        </div>
      </header>

      <main className="content list">
        {recordings.length === 0 ? (
          <div className="empty">
            <p>녹음이 없습니다.</p>
            <p className="muted">아래 버튼으로 녹음하거나, 설정에서 iCloud Drive를 연결해 Mac의 녹음을 가져오세요.</p>
          </div>
        ) : (
          <ul className="recordings">
            {recordings.map((r) => (
              <li key={r.id}>
                <button className="recording-row" onClick={() => onOpen(r.id)}>
                  <span className="row-title">
                    <span className="number">#{r.number}</span> {r.title}
                  </span>
                  <span className="row-meta">
                    <StatusChip item={r} />
                    {r.duration != null && <span>{formatTime(r.duration)}</span>}
                    <span>{new Date(r.createdAt).toLocaleDateString("ko-KR", { month: "short", day: "numeric" })}</span>
                    {!r.audioAvailable && r.status !== "recording" && <span className="muted">클라우드</span>}
                  </span>
                </button>
              </li>
            ))}
          </ul>
        )}
      </main>

      <div className="record-dock">
        <button className="record-button" onClick={onRecord} aria-label="녹음 시작">
          <span className="record-dot" />
        </button>
      </div>
    </div>
  );
}

function StatusChip({ item }: { item: RecordingItem }) {
  if (item.status === "recording") return <span className="chip rec">녹음 중</span>;
  if (item.hasTranscript) return <span className="chip done">전사</span>;
  return <span className="chip">전사 전</span>;
}

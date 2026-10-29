import { ReactNode, useEffect, useState } from "react";
import { api, formatTime, ProgressEvent, Recording, Status, SyncStatus } from "../api";
import { describe } from "./SyncSettings";
import { IS_MAC } from "../shortcuts";

const STATUS_LABEL: Record<Status, string> = {
  recording: "녹음 중",
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
  onSearch: () => void;
  onOpenSettings: () => void;
  recorder: ReactNode;
}

export function Sidebar({ recordings, selectedId, progress, onSelect, onImport, onSearch, onOpenSettings, recorder }: Props) {
  return (
    <aside className="sidebar">
      <header className="sidebar-header">
        <span className="brand">
          <img src="/favicon.svg" width="24" height="24" alt="" />
          sori
        </span>
        <div className="sidebar-actions">
          <button onClick={onSearch} title={`빠른 찾기 (${IS_MAC ? "⌘K" : "Ctrl+K"})`}>
            찾기
          </button>
          <button onClick={onImport} title={`가져오기 (${IS_MAC ? "⌘O" : "Ctrl+O"})`}>
            가져오기
          </button>
        </div>
      </header>
      {recorder}
      <ul className="recording-list">
        {recordings.map((r) => {
          const p = progress[r.id];
          return (
            <li key={r.id}>
              <button className={`recording-item ${r.id === selectedId ? "selected" : ""}`} onClick={() => onSelect(r.id)}>
                <span className="recording-title">
                  <span className="recording-number">#{r.number}</span>
                  {r.title}
                </span>
                <span className="recording-meta">
                  <span className={`badge badge-${r.status}`}>
                    {r.status === "processing" && p
                      ? p.stage === "waiting"
                        ? "대기열"
                        : `${Math.round(p.progress * 100)}%`
                      : STATUS_LABEL[r.status]}
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
        <SyncBadge />
      </footer>
    </aside>
  );
}

/** 동기화를 켰을 때만 보인다. 누르면 바로 동기화한다 */
function SyncBadge() {
  const [status, setStatus] = useState<SyncStatus | null>(null);
  useEffect(() => {
    api.syncStatus().then(setStatus);
    const unlisten = api.onSyncStatus(setStatus);
    return () => {
      unlisten.then((f) => f());
    };
  }, []);
  if (!status || status.provider === "local") return null;
  return (
    <button
      className={`ghost sync-badge ${status.error ? "error" : ""}`}
      title={status.error ?? "눌러서 지금 동기화"}
      disabled={status.running}
      onClick={() => api.syncNow()}
    >
      {status.running ? "동기화 중" : status.error ? "동기화 오류" : describe(status)}
    </button>
  );
}

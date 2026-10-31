import { ReactNode, useEffect, useMemo, useState } from "react";
import { api, formatTime, ProgressEvent, Recording, Status, SyncStatus } from "../api";
import { describe } from "./SyncSettings";
import { IS_MAC } from "../shortcuts";
import { buildTagTree, matchesTag } from "../tags";
import { TagTree } from "./TagTree";

const FILTER_KEY = "sori.tagFilter";

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
  const [filter, setFilter] = useState<string | null>(() => {
    try {
      return localStorage.getItem(FILTER_KEY);
    } catch {
      return null;
    }
  });
  const tree = useMemo(() => buildTagTree(recordings), [recordings]);
  const untagged = recordings.filter((r) => !r.tags?.length).length;
  // 고른 태그가 모든 녹음에서 사라졌으면 전체로 돌아간다
  const active = filter && recordings.some((r) => matchesTag(r.tags, filter)) ? filter : null;
  const shown = recordings.filter((r) => matchesTag(r.tags, active));
  const choose = (path: string | null) => {
    setFilter(path);
    try {
      if (path) localStorage.setItem(FILTER_KEY, path);
      else localStorage.removeItem(FILTER_KEY);
    } catch {
      // 저장하지 못해도 이번 실행에서는 걸러 보인다
    }
  };

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
      {tree.length > 0 && <TagTree tree={tree} total={recordings.length} untagged={untagged} selected={active} onSelect={choose} />}
      <ul className="recording-list">
        {shown.map((r) => {
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
                {!!r.tags?.length && (
                  <span className="recording-tags" title={r.tags.join(", ")}>
                    {r.tags.join(", ")}
                  </span>
                )}
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

/** 동기화를 켰을 때만 보인다. 상태 글과 "지금 동기화" 버튼. 버튼은 변경을 모으는 대기 없이 바로 맞춘다 */
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
    <div className="sync-badge">
      <span className={`sync-text ${status.error ? "error" : ""}`} title={status.error ?? undefined}>
        {status.running ? (status.message ?? "동기화 중") : status.error ? "동기화 오류" : describe(status)}
      </span>
      <button className="sync-now" disabled={status.running} onClick={() => api.syncNow()} title="iCloud Drive와 지금 맞춥니다">
        <span className={status.running ? "spin" : ""}>↻</span> 동기화
      </button>
    </div>
  );
}

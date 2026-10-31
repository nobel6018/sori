import { useMemo, useState } from "react";
import { formatTime } from "../../src/api";
import { buildTagTree, matchesTag, TagNode, UNTAGGED } from "../../src/tags";
import { RecordingItem, SyncStatus, syncLabel } from "./api";

const FILTER_KEY = "sori.tagFilter";

export function List({
  recordings,
  previewing,
  sync,
  onOpen,
  onSettings,
  onRecord,
  onRefresh,
}: {
  recordings: RecordingItem[];
  /** iPhone에서 미리 전사하고 있는 녹음 */
  previewing: string | null;
  sync: SyncStatus | null;
  onOpen: (id: string) => void;
  onSettings: () => void;
  onRecord: () => void;
  onRefresh: () => void;
}) {
  const [filter, setFilter] = useState<string | null>(() => localStorage.getItem(FILTER_KEY));
  const tree = useMemo(() => buildTagTree(recordings), [recordings]);
  // 고른 태그가 모든 녹음에서 사라졌으면 전체로 돌아간다
  const active = filter && recordings.some((r) => matchesTag(r.tags, filter)) ? filter : null;
  const shown = recordings.filter((r) => matchesTag(r.tags, active));
  const choose = (path: string | null) => {
    setFilter(path);
    if (path) localStorage.setItem(FILTER_KEY, path);
    else localStorage.removeItem(FILTER_KEY);
  };

  return (
    <div className="screen">
      <header className="bar">
        <h1>sori</h1>
        <div className="bar-actions">
          {sync && sync.provider !== "local" && (
            <button className={`sync-button ${sync.error ? "error" : ""}`} onClick={onRefresh} disabled={sync.running} aria-label="지금 동기화">
              <span className={sync.running ? "spin" : ""}>↻</span>
              <span className="sync-label">{syncLabel(sync)}</span>
            </button>
          )}
          <button className="text-button" onClick={onSettings}>
            설정
          </button>
        </div>
      </header>

      {tree.length > 0 && <TagBar tree={tree} recordings={recordings} active={active} onChoose={choose} />}

      <main className="content list">
        {recordings.length === 0 ? (
          <div className="empty">
            <p>녹음이 없습니다.</p>
            <p className="muted">아래 버튼으로 녹음하거나, 설정에서 iCloud Drive를 연결해 Mac의 녹음을 가져오세요.</p>
          </div>
        ) : (
          <ul className="recordings">
            {shown.map((r) => (
              <li key={r.id}>
                <button className="recording-row" onClick={() => onOpen(r.id)}>
                  <span className="row-title">
                    <span className="number">#{r.number}</span> {r.title}
                  </span>
                  <span className="row-meta">
                    <StatusChip item={r} previewing={previewing === r.id} />
                    {r.duration != null && <span>{formatTime(r.duration)}</span>}
                    <span>{new Date(r.createdAt).toLocaleDateString("ko-KR", { month: "short", day: "numeric" })}</span>
                    {!r.audioAvailable && r.status !== "recording" && <span className="muted">클라우드</span>}
                  </span>
                  {!!r.tags?.length && <span className="row-tags">{r.tags.join(", ")}</span>}
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

/** 목록 위 태그 칩 줄. 하위 태그가 있는 태그를 고르면 그 안으로 들어가 하위 태그를 보여 준다(폴더처럼)
 * @see docs/organize.md */
function TagBar({
  tree,
  recordings,
  active,
  onChoose,
}: {
  tree: TagNode[];
  recordings: RecordingItem[];
  active: string | null;
  onChoose: (path: string | null) => void;
}) {
  // 지금 보고 있는 단계: 고른 태그에 하위가 있으면 그 안, 없으면 그 부모
  const find = (nodes: TagNode[], path: string): TagNode | undefined => {
    for (const n of nodes) {
      if (n.path === path) return n;
      if (path.startsWith(n.path + "/")) return find(n.children, path);
    }
  };
  const current = active && active !== UNTAGGED ? find(tree, active) : undefined;
  const levelPath = current ? (current.children.length ? current.path : current.path.split("/").slice(0, -1).join("/")) : "";
  const level = levelPath ? (find(tree, levelPath)?.children ?? []) : tree;
  const parent = levelPath ? levelPath.split("/").slice(0, -1).join("/") || null : null;
  const untagged = recordings.filter((r) => !r.tags?.length).length;

  return (
    <nav className="tag-bar" aria-label="태그">
      {levelPath ? (
        <>
          <button className="tag-pill" onClick={() => onChoose(parent)}>
            ‹ {parent ? parent.split("/").pop() : "전체"}
          </button>
          <button className={`tag-pill ${active === levelPath ? "on" : ""}`} onClick={() => onChoose(levelPath)}>
            {levelPath.split("/").pop()} 전체
          </button>
        </>
      ) : (
        <button className={`tag-pill ${active === null ? "on" : ""}`} onClick={() => onChoose(null)}>
          전체 {recordings.length}
        </button>
      )}
      {level.map((n) => (
        <button key={n.path} className={`tag-pill ${active === n.path ? "on" : ""}`} onClick={() => onChoose(active === n.path ? levelPath || null : n.path)}>
          {n.name} {n.count}
          {n.children.length > 0 && " ›"}
        </button>
      ))}
      {!levelPath && untagged > 0 && untagged < recordings.length && (
        <button className={`tag-pill muted ${active === UNTAGGED ? "on" : ""}`} onClick={() => onChoose(active === UNTAGGED ? null : UNTAGGED)}>
          태그 없음 {untagged}
        </button>
      )}
    </nav>
  );
}

function StatusChip({ item, previewing }: { item: RecordingItem; previewing: boolean }) {
  if (item.status === "recording") return <span className="chip rec">녹음 중</span>;
  if (item.hasTranscript) return <span className="chip done">전사</span>;
  if (previewing) return <span className="chip">iPhone 전사 중</span>;
  if (item.hasPreview) return <span className="chip">미리 전사</span>;
  return <span className="chip">전사 전</span>;
}

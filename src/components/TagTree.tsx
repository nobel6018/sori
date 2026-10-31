import { useState } from "react";
import { TagNode, UNTAGGED } from "../tags";

interface Props {
  tree: TagNode[];
  total: number;
  untagged: number;
  /** 고른 태그 경로. null이면 전체, UNTAGGED면 태그 없는 녹음 */
  selected: string | null;
  onSelect: (path: string | null) => void;
}

/** 사이드바의 태그 트리. `/`로 나눈 태그를 폴더처럼 펼친다. 상위를 고르면 하위 태그 녹음도 보인다
 * @see docs/organize.md */
export function TagTree({ tree, total, untagged, selected, onSelect }: Props) {
  // 처음에는 첫 단계만 보이고, 고른 태그의 상위는 펼쳐 둔다
  const [open, setOpen] = useState<Set<string>>(() => new Set(ancestors(selected)));
  const toggle = (path: string) =>
    setOpen((prev) => {
      const next = new Set(prev);
      if (next.has(path)) next.delete(path);
      else next.add(path);
      return next;
    });

  const row = (node: TagNode, depth: number) => (
    <li key={node.path}>
      <div className={`tag-row ${selected === node.path ? "selected" : ""}`} style={{ paddingLeft: 6 + depth * 14 }}>
        {node.children.length > 0 ? (
          <button className="tag-toggle" aria-label={open.has(node.path) ? "접기" : "펼치기"} onClick={() => toggle(node.path)}>
            {open.has(node.path) ? "▾" : "▸"}
          </button>
        ) : (
          <span className="tag-toggle" />
        )}
        <button className="tag-name" onClick={() => onSelect(selected === node.path ? null : node.path)} title={node.path}>
          {node.name}
        </button>
        <span className="tag-count">{node.count}</span>
      </div>
      {open.has(node.path) && node.children.length > 0 && <ul>{node.children.map((c) => row(c, depth + 1))}</ul>}
    </li>
  );

  return (
    <nav className="tag-tree" aria-label="태그">
      <ul>
        <li>
          <div className={`tag-row ${selected === null ? "selected" : ""}`} style={{ paddingLeft: 6 }}>
            <span className="tag-toggle" />
            <button className="tag-name" onClick={() => onSelect(null)}>
              전체
            </button>
            <span className="tag-count">{total}</span>
          </div>
        </li>
        {tree.map((n) => row(n, 0))}
        {untagged > 0 && untagged < total && (
          <li>
            <div className={`tag-row ${selected === UNTAGGED ? "selected" : ""}`} style={{ paddingLeft: 6 }}>
              <span className="tag-toggle" />
              <button className="tag-name muted" onClick={() => onSelect(selected === UNTAGGED ? null : UNTAGGED)}>
                태그 없음
              </button>
              <span className="tag-count">{untagged}</span>
            </div>
          </li>
        )}
      </ul>
    </nav>
  );
}

function ancestors(path: string | null): string[] {
  if (!path || path === UNTAGGED) return [];
  const parts = path.split("/");
  return parts.slice(0, -1).map((_, i) => parts.slice(0, i + 1).join("/"));
}

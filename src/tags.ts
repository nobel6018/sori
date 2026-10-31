// 녹음 태그 모아 보기. `/`로 계층을 만든 태그(업무/영업/한빛로보틱스)를 폴더 트리처럼 보여 준다.
// 데스크톱(Sidebar, RecordingView)과 iPhone 앱(mobile/src)이 함께 쓴다.
// 태그 정리 규칙은 Rust(crates/sori-sync/src/tags.rs)와 같다
// @see docs/organize.md

export interface TagNode {
  /** 전체 경로(업무/영업) */
  path: string;
  /** 마지막 단계 이름(영업) */
  name: string;
  /** 이 태그나 하위 태그가 붙은 녹음 수 */
  count: number;
  children: TagNode[];
}

/** 태그 없는 녹음만 보는 필터 값 */
export const UNTAGGED = "\u0000untagged";

/** 태그 하나를 정리한다. 앞뒤 공백, `/` 앞뒤 공백, 빈 단계, `#` 머리를 없앤다 */
export function normalizeTag(raw: string): string | null {
  const parts = raw
    .trim()
    .replace(/^#+/, "")
    .split("/")
    .map((p) => p.split(/\s+/).filter(Boolean).join(" "))
    .filter(Boolean);
  return parts.length ? parts.join("/").slice(0, 60) : null;
}

/** 태그 목록을 정리한다. 순서는 지키고 같은 태그(대소문자 무시)는 처음 것만 남긴다 */
export function normalizeTags(raw: string[]): string[] {
  const out: string[] = [];
  for (const t of raw) {
    const tag = normalizeTag(t);
    if (tag && !out.some((o) => o.toLowerCase() === tag.toLowerCase())) out.push(tag);
  }
  return out.slice(0, 10);
}

/** 이 녹음이 필터(태그 경로)에 들어가는지. 하위 태그가 붙은 녹음도 들어간다 */
export function matchesTag(tags: string[] | undefined, filter: string | null): boolean {
  if (!filter) return true;
  if (filter === UNTAGGED) return !tags?.length;
  return (tags ?? []).some((t) => t === filter || t.startsWith(filter + "/"));
}

/** 녹음들의 태그로 트리를 만든다. 형제는 녹음 수가 많은 순, 같으면 이름순 */
export function buildTagTree(recordings: { tags?: string[] }[]): TagNode[] {
  const root: TagNode = { path: "", name: "", count: 0, children: [] };
  for (const r of recordings) {
    // 한 녹음이 같은 상위 태그를 여러 번 세지 않도록 경로를 모아 한 번씩 센다
    const paths = new Set<string>();
    for (const tag of r.tags ?? []) {
      const parts = tag.split("/");
      parts.forEach((_, i) => paths.add(parts.slice(0, i + 1).join("/")));
    }
    for (const path of paths) {
      let node = root;
      for (const name of path.split("/")) {
        const childPath = node.path ? `${node.path}/${name}` : name;
        let child = node.children.find((c) => c.path === childPath);
        if (!child) {
          child = { path: childPath, name, count: 0, children: [] };
          node.children.push(child);
        }
        node = child;
      }
      node.count += 1;
    }
  }
  const sort = (nodes: TagNode[]) => {
    nodes.sort((a, b) => b.count - a.count || a.name.localeCompare(b.name, "ko"));
    nodes.forEach((n) => sort(n.children));
  };
  sort(root.children);
  return root.children;
}

/** 쓰인 태그와 그 상위 태그 경로 전부. 이름순. 질문 범위 고르기에 쓴다 */
export function allTagPaths(recordings: { tags?: string[] }[]): string[] {
  const paths = new Set<string>();
  for (const r of recordings)
    for (const tag of r.tags ?? []) {
      const parts = tag.split("/");
      parts.forEach((_, i) => paths.add(parts.slice(0, i + 1).join("/")));
    }
  return [...paths].sort((a, b) => a.localeCompare(b, "ko"));
}

/** 쓰인 태그와 녹음 수. 자동완성 후보로 쓴다. 많이 쓴 순 */
export function tagCounts(recordings: { tags?: string[] }[]): [string, number][] {
  const counts = new Map<string, number>();
  for (const r of recordings) for (const t of r.tags ?? []) counts.set(t, (counts.get(t) ?? 0) + 1);
  return [...counts].sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0], "ko"));
}

const INITIALS = "ㄱㄲㄴㄷㄸㄹㅁㅂㅃㅅㅆㅇㅈㅉㅊㅋㅌㅍㅎ";

function initialOf(c: string): string | null {
  const code = c.charCodeAt(0);
  return code >= 0xac00 && code <= 0xd7a3 ? INITIALS[Math.floor((code - 0xac00) / 588)] : null;
}

/** 초성을 섞은 검색어로 부분 문자열을 찾는다(ㅎㅂ, 박ㅅ). 대소문자와 띄어쓰기는 무시한다 */
export function matchesQuery(text: string, query: string): boolean {
  const hay = [...text.toLowerCase().replace(/\s+/g, "")];
  const needle = [...query.toLowerCase().replace(/\s+/g, "")];
  if (!needle.length) return true;
  for (let i = 0; i + needle.length <= hay.length; i++) {
    if (needle.every((n, j) => hay[i + j] === n || (INITIALS.includes(n) && initialOf(hay[i + j]) === n))) return true;
  }
  return false;
}

/** 태그 입력 자동완성 후보. 이미 붙은 태그는 빼고 많이 쓴 순으로 */
export function suggestTags(all: [string, number][], query: string, current: string[], limit = 8): string[] {
  return all
    .map(([t]) => t)
    .filter((t) => !current.includes(t) && matchesQuery(t, query))
    .slice(0, limit);
}

/** 지도 앱에서 여는 주소. Apple 지도는 macOS, iOS 모두에서 열린다 */
export function mapUrl(loc: { lat: number; lon: number; place?: string | null }): string {
  const q = encodeURIComponent(loc.place || "녹음한 곳");
  return `https://maps.apple.com/?ll=${loc.lat},${loc.lon}&q=${q}`;
}

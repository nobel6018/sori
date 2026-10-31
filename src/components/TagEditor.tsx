import { useMemo, useState } from "react";
import { normalizeTag, suggestTags } from "../tags";

interface Props {
  tags: string[];
  /** 쓰인 태그와 녹음 수(tagCounts). 자동완성 후보 */
  allTags: [string, number][];
  onChange: (tags: string[]) => void;
  /** AI가 붙인 뒤 사람이 고치지 않았다 */
  aiTagged?: boolean;
}

/** 녹음 태그 편집. 칩으로 보여 주고, 입력하면 쓰던 태그를 초성까지 찾아 제안한다. 데스크톱과 iPhone 앱이 함께 쓴다
 * @see docs/organize.md */
export function TagEditor({ tags, allTags, onChange, aiTagged }: Props) {
  const [query, setQuery] = useState("");
  const [focused, setFocused] = useState(false);
  /** 화살표로 고른 제안. -1이면 고르지 않았고 Enter는 친 글자 그대로 붙인다 */
  const [active, setActive] = useState(-1);
  const suggestions = useMemo(() => suggestTags(allTags, query, tags), [allTags, query, tags]);
  const showList = focused && suggestions.length > 0;

  const add = (raw: string) => {
    const tag = normalizeTag(raw);
    setQuery("");
    setActive(-1);
    if (tag && !tags.some((t) => t.toLowerCase() === tag.toLowerCase())) onChange([...tags, tag]);
  };

  return (
    <div className="tag-editor">
      {tags.map((t) => (
        <span key={t} className="tag-chip">
          {t}
          <button type="button" aria-label={`${t} 태그 빼기`} onClick={() => onChange(tags.filter((x) => x !== t))}>
            ×
          </button>
        </span>
      ))}
      <span className="tag-input-wrap">
        <input
          className="tag-input"
          value={query}
          placeholder={tags.length ? "태그 추가" : "태그 추가 (업무/영업처럼 /로 나눔)"}
          aria-label="태그 추가"
          enterKeyHint="done"
          onChange={(e) => {
            setQuery(e.target.value);
            setActive(-1);
          }}
          onFocus={() => setFocused(true)}
          // 제안을 누르는 동안 목록이 사라지지 않도록 조금 늦게 닫는다
          onBlur={() => setTimeout(() => setFocused(false), 150)}
          onKeyDown={(e) => {
            // WebKit은 한글 조합을 확정하는 Enter를 isComposing 없이 keyCode 229로 보낸다
            if (e.nativeEvent.isComposing || e.keyCode === 229) return;
            if (e.key === "ArrowDown" && showList) {
              e.preventDefault();
              setActive((a) => Math.min(a + 1, suggestions.length - 1));
            } else if (e.key === "ArrowUp" && showList) {
              e.preventDefault();
              setActive((a) => Math.max(a - 1, -1));
            } else if (e.key === "Enter" || e.key === ",") {
              e.preventDefault();
              // 화살표로 고른 제안이 있으면 그것, 없으면 친 글자 그대로(새 태그)
              add(showList && active >= 0 && e.key === "Enter" ? suggestions[active] : query);
            } else if (e.key === "Backspace" && !query && tags.length) {
              onChange(tags.slice(0, -1));
            } else if (e.key === "Escape") {
              setQuery("");
            }
          }}
        />
        {showList && (
          <ul className="tag-suggestions" role="listbox">
            {suggestions.map((s, i) => (
              <li key={s}>
                <button
                  type="button"
                  role="option"
                  aria-selected={i === active}
                  className={i === active ? "active" : ""}
                  onMouseDown={(e) => e.preventDefault()}
                  onClick={() => add(s)}
                >
                  {s}
                </button>
              </li>
            ))}
          </ul>
        )}
      </span>
      {aiTagged && tags.length > 0 && <span className="tag-ai" title="Mac의 AI가 붙인 태그입니다. 고치면 다시 덮어쓰지 않습니다">AI</span>}
    </div>
  );
}

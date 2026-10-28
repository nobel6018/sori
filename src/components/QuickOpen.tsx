import { KeyboardEvent, useEffect, useMemo, useRef, useState } from "react";
import Markdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { api, formatTime, RecordingDetail, RecordingHits, speakerName } from "../api";
import { recordingMarkdown, toBlocks, transcriptMarkdown } from "../markdown";
import { IS_MAC } from "../shortcuts";

interface Props {
  initialQuery: string;
  onClose: () => void;
  onOpen: (id: string, time: number | null) => void;
}

/** 목록의 한 줄. 녹음 자체이거나, 그 녹음 안에서 찾은 문장이다 */
interface Row {
  key: string;
  group: RecordingHits;
  /** 찾은 문장의 순서. 녹음 줄이면 -1 */
  hit: number;
}

const MOD = IS_MAC ? "⌘" : "Ctrl+";

/**
 * 녹음 목록과 전사 검색을 한 창에서 한다. 왼쪽에서 고르면 오른쪽에 요약과 전사를 미리 보여 주고,
 * 열지 않고도 마크다운으로 복사할 수 있다
 */
export function QuickOpen({ initialQuery, onClose, onOpen }: Props) {
  const [query, setQuery] = useState(initialQuery);
  const [results, setResults] = useState<RecordingHits[] | null>(null);
  const [cursor, setCursor] = useState(0);
  const [detail, setDetail] = useState<RecordingDetail | null>(null);
  const [copied, setCopied] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const listRef = useRef<HTMLUListElement>(null);

  // 글자를 칠 때마다 모든 전사를 뒤지지 않도록 잠깐 멈췄을 때 찾는다
  useEffect(() => {
    let stale = false;
    const timer = setTimeout(
      () => {
        api
          .searchRecordings(query)
          .then((r) => {
            if (stale) return;
            setResults(r);
            setCursor(0);
          })
          .catch((e) => !stale && setError(String(e)));
      },
      query ? 150 : 0,
    );
    return () => {
      stale = true;
      clearTimeout(timer);
    };
  }, [query]);

  const rows = useMemo(() => {
    const out: Row[] = [];
    for (const group of results ?? []) {
      out.push({ key: group.recording.id, group, hit: -1 });
      group.hits.forEach((_, i) => out.push({ key: `${group.recording.id}-${i}`, group, hit: i }));
    }
    return out;
  }, [results]);

  const row = rows[cursor] as Row | undefined;
  const selectedId = row?.group.recording.id ?? null;

  useEffect(() => {
    if (!selectedId) {
      setDetail(null);
      return;
    }
    let stale = false;
    api.getRecording(selectedId).then((d) => !stale && setDetail(d)).catch(() => !stale && setDetail(null));
    return () => {
      stale = true;
    };
  }, [selectedId]);

  useEffect(() => {
    listRef.current?.querySelector(`[data-row="${cursor}"]`)?.scrollIntoView({ block: "nearest" });
  }, [cursor]);

  const hitTime = (r: Row | undefined) => (r && r.hit >= 0 ? r.group.hits[r.hit].start : null);

  const copy = async (what: "markdown" | "summary" | "transcript") => {
    if (!detail || detail.recording.id !== selectedId) return;
    const text =
      what === "markdown"
        ? recordingMarkdown(detail)
        : what === "summary"
          ? (detail.summary?.markdown ?? "")
          : transcriptMarkdown(detail.recording, detail.transcript?.utterances ?? []);
    if (!text) return;
    await navigator.clipboard.writeText(text);
    setCopied(what);
    setTimeout(() => setCopied(null), 1500);
  };

  const onKeyDown = (e: KeyboardEvent<HTMLInputElement>) => {
    if (e.nativeEvent.isComposing) return;
    const mod = IS_MAC ? e.metaKey : e.ctrlKey;
    if (e.key === "Escape") onClose();
    else if (e.key === "ArrowDown") setCursor((c) => Math.min(c + 1, rows.length - 1));
    else if (e.key === "ArrowUp") setCursor((c) => Math.max(c - 1, 0));
    else if (e.key === "Enter" && row) onOpen(row.group.recording.id, hitTime(row));
    else if (mod && e.shiftKey && e.key.toLowerCase() === "c") copy("markdown");
    else if (mod && e.shiftKey && e.key.toLowerCase() === "s") copy("summary");
    else if (mod && e.shiftKey && e.key.toLowerCase() === "t") copy("transcript");
    else return;
    e.preventDefault();
  };

  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div className="quick-open" role="dialog" aria-label="빠른 찾기" onClick={(e) => e.stopPropagation()}>
        <input
          className="quick-input"
          autoFocus
          placeholder="녹음 제목, 요약, 전사에서 찾기"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          onKeyDown={onKeyDown}
          aria-label="검색어"
        />
        <div className="quick-body">
          <ul className="quick-list" ref={listRef} role="listbox">
            {results === null && <li className="muted quick-empty">찾는 중</li>}
            {results !== null && rows.length === 0 && (
              <li className="muted quick-empty">{query ? "찾은 내용이 없습니다" : "녹음이 없습니다"}</li>
            )}
            {rows.map((r, i) => {
              const rec = r.group.recording;
              const hit = r.hit >= 0 ? r.group.hits[r.hit] : null;
              return (
                <li
                  key={r.key}
                  data-row={i}
                  role="option"
                  aria-selected={i === cursor}
                  className={`quick-row ${hit ? "hit" : "recording"} ${i === cursor ? "selected" : ""}`}
                  onMouseMove={() => i !== cursor && setCursor(i)}
                  onClick={() => onOpen(rec.id, hitTime(r))}
                >
                  {hit ? (
                    <>
                      <span className="quick-hit-meta">
                        {hit.kind === "summary" ? "요약" : `${formatTime(hit.start ?? 0)} ${speakerName(rec, hit.speaker ?? 0)}`}
                      </span>
                      <span className="quick-hit-text">
                        <Highlight text={hit.text} query={query} />
                      </span>
                    </>
                  ) : (
                    <>
                      <span className="quick-title">
                        <Highlight text={rec.title} query={r.group.titleMatch ? query : ""} />
                      </span>
                      <span className="quick-meta">
                        {new Date(rec.createdAt).toLocaleDateString("ko-KR")}
                        {rec.duration != null && `, ${formatTime(rec.duration)}`}
                        {r.group.total > r.group.hits.length && `, ${r.group.total}곳 중 ${r.group.hits.length}곳`}
                      </span>
                    </>
                  )}
                </li>
              );
            })}
          </ul>
          <Preview detail={detail?.recording.id === selectedId ? detail : null} focusTime={hitTime(row)} />
        </div>
        <footer className="quick-footer">
          {error && <span className="error-text">{error}</span>}
          <span>↩ 열기</span>
          <button className="ghost" onClick={() => copy("markdown")} disabled={!detail?.transcript}>
            {copied === "markdown" ? "복사했습니다" : `마크다운 복사 ⇧${MOD}C`}
          </button>
          <button className="ghost" onClick={() => copy("summary")} disabled={!detail?.summary}>
            {copied === "summary" ? "복사했습니다" : `요약 복사 ⇧${MOD}S`}
          </button>
          <button className="ghost" onClick={() => copy("transcript")} disabled={!detail?.transcript}>
            {copied === "transcript" ? "복사했습니다" : `전사 복사 ⇧${MOD}T`}
          </button>
        </footer>
      </div>
    </div>
  );
}

/** 요약 전체와, 찾은 문장 근처의 전사를 보여 준다 */
function Preview({ detail, focusTime }: { detail: RecordingDetail | null; focusTime: number | null }) {
  const ref = useRef<HTMLDivElement>(null);
  const blocks = useMemo(() => toBlocks(detail?.transcript?.utterances ?? []), [detail]);
  // 1시간 회의 전사를 전부 그리면 커서를 옮길 때마다 느려진다. 찾은 곳 앞뒤만 그린다
  const focus = focusTime == null ? 0 : Math.max(0, blocks.findIndex((b) => b.end > focusTime));
  const shown = blocks.slice(Math.max(0, focus - 3), focus + 12);

  useEffect(() => {
    if (focusTime == null) ref.current?.scrollTo({ top: 0 });
    else ref.current?.querySelector(".preview-focus")?.scrollIntoView({ block: "center" });
  }, [detail, focusTime]);

  if (!detail) return <div className="quick-preview" />;
  const { recording, transcript, summary } = detail;
  return (
    <div className="quick-preview" ref={ref}>
      <h3>{recording.title}</h3>
      {!transcript && <p className="muted">아직 받아 적지 않은 녹음입니다.</p>}
      {summary && focusTime == null && (
        <section className="summary-body preview-summary">
          <Markdown remarkPlugins={[remarkGfm]}>{summary.markdown}</Markdown>
        </section>
      )}
      {transcript && !summary && focusTime == null && <p className="muted">요약이 없습니다. 전사 앞부분입니다.</p>}
      {shown.map((b) => {
        const isFocus = focusTime != null && b.start <= focusTime && focusTime < b.end + 0.01;
        return (
          <p key={b.start} className={`preview-block ${isFocus ? "preview-focus" : ""}`}>
            <span className={`speaker-name spk-${b.speaker % 8}`}>{speakerName(recording, b.speaker)}</span>{" "}
            <span className="muted">{formatTime(b.start)}</span>
            <br />
            {b.utterances.map((u) => u.text.trim()).join(" ")}
          </p>
        );
      })}
    </div>
  );
}

/** 띄어쓰기를 무시하고 찾으므로, 화면에서도 띄어쓰기를 건너뛰며 맞는 곳을 칠한다 */
function Highlight({ text, query }: { text: string; query: string }) {
  const needle = [...query.toLowerCase()].filter((c) => !/\s/.test(c));
  if (needle.length === 0) return <>{text}</>;
  const chars = [...text];
  const lower = chars.map((c) => c.toLowerCase());
  for (let start = 0; start < chars.length; start++) {
    if (/\s/.test(lower[start])) continue;
    let i = start;
    let n = 0;
    while (i < chars.length && n < needle.length) {
      if (/\s/.test(lower[i])) i++;
      else if (lower[i] === needle[n]) {
        i++;
        n++;
      } else break;
    }
    if (n === needle.length) {
      return (
        <>
          {chars.slice(0, start).join("")}
          <mark>{chars.slice(start, i).join("")}</mark>
          {chars.slice(i).join("")}
        </>
      );
    }
  }
  return <>{text}</>;
}

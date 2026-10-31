import { useEffect, useRef, useState } from "react";
import Markdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { Answer, api, formatTime, Provider, PROVIDER_LABEL, ProviderStatus } from "../api";

interface Props {
  /** 없으면 모든 녹음에 묻는다 */
  recordingId: string | null;
  /** 답의 시각을 눌렀을 때. 전체 질문이면 녹음 id가 함께 온다 */
  onCite: (recordingId: string | null, time: number | null) => void;
  /** 전체 질문의 범위로 고를 태그 경로(allTagPaths). 처음에는 사이드바에서 보던 태그를 고른다 */
  tagPaths?: string[];
}

interface Turn {
  question: string;
  answer: Answer | null;
  error: string | null;
}

/** 대괄호 안의 인용 하나: "12:34", "105:30", "1:02:03", "R3 12:34", "R3" */
const CITE_PART = /^(R\d+)?\s*(\d+:\d{2}(?::\d{2})?)?$/;

function toSeconds(stamp: string): number {
  return stamp.split(":").reduce((n, part) => n * 60 + Number(part), 0);
}

/**
 * 답의 시각 표시를 sori-cite: 링크로 바꿔, 마크다운 링크 렌더러에서 눌리게 한다.
 * "[R3 12:34, R5 01:10]"처럼 한 괄호에 여러 개를 묶어 써도 하나씩 링크로 만든다.
 * 뒤에 (가 붙은 대괄호는 원래 마크다운 링크라 건드리지 않는다
 */
export function linkCitations(markdown: string): string {
  return markdown.replace(/\[([^\[\]\n]{1,80})\](?!\()/g, (whole, inner: string) => {
    const parts = inner.split(/[,;]\s*/).map((p) => p.trim().match(CITE_PART));
    if (parts.some((m) => !m || (!m[1] && !m[2]))) return whole;
    return parts
      .map((m) => {
        const [, key, stamp] = m!;
        const label = [key, stamp].filter(Boolean).join(" ");
        return `[${label}](sori-cite:${key ?? ""}/${stamp ? toSeconds(stamp) : ""})`;
      })
      .join(" ");
  });
}

const EXAMPLES_ONE = ["결정된 것만 알려 줘", "내가 맡은 일이 뭐야?", "가장 의견이 갈린 주제는?"];
const EXAMPLES_ALL = ["지난주 회의에서 배포 날짜를 언제로 정했지?", "민지가 말한 걱정거리를 모아 줘", "아직 안 끝난 할 일이 뭐야?"];

export function AskPanel({ recordingId, onCite, tagPaths = [] }: Props) {
  const [scope, setScope] = useState<string | null>(() => {
    try {
      const seen = localStorage.getItem("sori.tagFilter");
      return seen && tagPaths.includes(seen) ? seen : null;
    } catch {
      return null;
    }
  });
  const [providers, setProviders] = useState<ProviderStatus[] | null>(null);
  const [provider, setProvider] = useState<Provider | null>(null);
  const [question, setQuestion] = useState("");
  const [turns, setTurns] = useState<Turn[]>([]);
  const [running, setRunning] = useState(false);
  const endRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    Promise.all([api.summaryProviders(), api.getSettings()]).then(([list, settings]) => {
      setProviders(list);
      const installed = list.filter((p) => p.path).map((p) => p.provider);
      const remembered = settings.summaryProvider;
      setProvider(remembered && installed.includes(remembered) ? remembered : (installed[0] ?? null));
    });
  }, []);

  useEffect(() => {
    endRef.current?.scrollIntoView({ block: "end" });
  }, [turns]);

  // 창을 닫거나 화면이 바뀌면 답을 받을 곳이 없어진다. 기다리던 질문은 취소해, 다시 열었을 때 막히지 않게 한다
  const runningRef = useRef(false);
  runningRef.current = running;
  useEffect(
    () => () => {
      if (runningRef.current) api.cancelAsk(recordingId);
    },
    [recordingId],
  );

  const ask = async (q: string) => {
    const text = q.trim();
    if (!text || !provider || running) return;
    setQuestion("");
    setRunning(true);
    setTurns((prev) => [...prev, { question: text, answer: null, error: null }]);
    const settle = (patch: Partial<Turn>) =>
      setTurns((prev) => prev.map((t, i) => (i === prev.length - 1 ? { ...t, ...patch } : t)));
    try {
      settle({ answer: await api.ask(text, recordingId, provider, recordingId ? null : scope) });
    } catch (e) {
      settle({ error: String(e) });
    } finally {
      setRunning(false);
    }
  };

  const installed = providers?.filter((p) => p.path) ?? [];

  return (
    <section className="ask">
      <div className="ask-turns">
        {turns.length === 0 && (
          <div className="ask-empty">
            <p className="muted">
              {recordingId
                ? "이 녹음의 전사, 요약, 메모를 근거로 답합니다. 답의 시각을 누르면 그 부분으로 갑니다."
                : "모든 녹음의 요약과, 질문 낱말이 나온 발화를 근거로 답합니다. 답의 녹음과 시각을 누르면 그 녹음을 엽니다."}
            </p>
            <div className="chips">
              {(recordingId ? EXAMPLES_ONE : EXAMPLES_ALL).map((ex) => (
                <button key={ex} className="chip" onClick={() => ask(ex)} disabled={!provider}>
                  {ex}
                </button>
              ))}
            </div>
          </div>
        )}
        {turns.map((t, i) => (
          <div key={i} className="ask-turn">
            <p className="ask-question">{t.question}</p>
            {t.error && <div className="error">{t.error}</div>}
            {!t.answer && !t.error && <p className="muted">답을 찾는 중입니다. 30초에서 1분쯤 걸립니다.</p>}
            {t.answer && (
              <div className="ask-answer summary-body">
                <Markdown
                  remarkPlugins={[remarkGfm]}
                  // 답은 LLM이 쓴 글이라, 외부 이미지를 그리면 그 주소로 요청이 나간다
                  disallowedElements={["img"]}
                  urlTransform={(url) => url}
                  components={{
                    a: ({ href, children }) => {
                      // 답에 섞인 일반 링크는 누르면 앱 화면이 그 페이지로 바뀌므로 글자로만 보여 준다
                      if (!href?.startsWith("sori-cite:")) return <span title={href}>{children}</span>;
                      const [key, secs] = href.slice("sori-cite:".length).split("/");
                      const source = t.answer!.sources.find((s) => s.key === key);
                      const time = secs ? formatTime(Number(secs)) : "";
                      return (
                        <button
                          className="cite"
                          title={source?.title}
                          onClick={() => onCite(source?.recordingId ?? null, secs ? Number(secs) : null)}
                        >
                          {source ? [source.title, time].filter(Boolean).join(" ") : time}
                        </button>
                      );
                    },
                  }}
                >
                  {linkCitations(t.answer.markdown)}
                </Markdown>
              </div>
            )}
          </div>
        ))}
        <div ref={endRef} />
      </div>
      <form
        className="ask-form"
        onSubmit={(e) => {
          e.preventDefault();
          ask(question);
        }}
      >
        <input
          value={question}
          onChange={(e) => setQuestion(e.target.value)}
          placeholder={recordingId ? "이 녹음에 물어보기" : scope ? `'${scope}' 녹음에 물어보기` : "모든 녹음에 물어보기"}
          aria-label="질문"
          disabled={!provider}
        />
        {!recordingId && tagPaths.length > 0 && (
          <select
            aria-label="물어볼 녹음 범위"
            title="태그로 좁히면 오래된 녹음이 글자 수 상한에 잘리지 않습니다"
            value={scope ?? ""}
            onChange={(e) => setScope(e.target.value || null)}
            disabled={running}
          >
            <option value="">모든 녹음</option>
            {tagPaths.map((t) => (
              <option key={t} value={t}>
                {t}
              </option>
            ))}
          </select>
        )}
        <select aria-label="답할 서비스" value={provider ?? ""} onChange={(e) => setProvider(e.target.value as Provider)} disabled={running}>
          {installed.map((p) => (
            <option key={p.provider} value={p.provider}>
              {PROVIDER_LABEL[p.provider]}
            </option>
          ))}
        </select>
        {running ? (
          <button type="button" onClick={() => api.cancelAsk(recordingId)}>
            취소
          </button>
        ) : (
          <button className="primary" type="submit" disabled={!provider || !question.trim()}>
            묻기
          </button>
        )}
      </form>
      {providers !== null && installed.length === 0 && (
        <p className="error">claude 또는 codex CLI가 있어야 물어볼 수 있습니다.</p>
      )}
    </section>
  );
}

import { useEffect, useState } from "react";
import Markdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { api, Provider, PROVIDER_LABEL, ProviderStatus, Summary, SummaryTemplate } from "../api";
import { useCommands } from "../shortcuts";

interface Props {
  recordingId: string;
  /** 오래된 것부터 */
  summaries: Summary[];
  onSummarized: () => Promise<void>;
}

export function SummaryPanel({ recordingId, summaries, onSummarized }: Props) {
  const [providers, setProviders] = useState<ProviderStatus[] | null>(null);
  const [provider, setProvider] = useState<Provider | null>(null);
  const [templates, setTemplates] = useState<SummaryTemplate[]>([]);
  const [template, setTemplate] = useState<string | null>(null);
  const [running, setRunning] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  /** 보고 있는 요약. 없으면 가장 최근 것 */
  const [shown, setShown] = useState<string | null>(null);

  useEffect(() => {
    Promise.all([api.summaryProviders(), api.getSettings()]).then(([list, settings]) => {
      setProviders(list);
      const installed = list.filter((p) => p.path).map((p) => p.provider);
      const remembered = settings.summaryProvider;
      setProvider(remembered && installed.includes(remembered) ? remembered : (installed[0] ?? null));
      setTemplates(settings.summaryTemplates);
      setTemplate(settings.defaultTemplate);
    });
  }, []);

  // 자동 요약처럼 이 화면 밖에서 시작한 요약도 진행 중으로 보여 준다
  useEffect(() => {
    api.summaryRunning(recordingId).then(setRunning);
    const unlisten = api.onSummaryState((e) => {
      if (e.id !== recordingId) return;
      setRunning(e.running);
      if (!e.running) onSummarized();
    });
    return () => {
      unlisten.then((f) => f());
    };
  }, [recordingId, onSummarized]);

  const run = async () => {
    if (!provider) return;
    setError(null);
    setShown(null);
    try {
      await api.summarize(recordingId, provider, template);
    } catch (e) {
      setError(String(e));
    }
  };

  useCommands((command) => {
    if (command === "summarize" && !running && provider) run();
    if (command === "cancel" && running) api.cancelSummary(recordingId);
  });

  const current = summaries.find((s) => s.id === shown) ?? summaries[summaries.length - 1] ?? null;

  const copy = async () => {
    if (!current) return;
    await navigator.clipboard.writeText(current.markdown);
    setCopied(true);
    setTimeout(() => setCopied(false), 1500);
  };

  const remove = async () => {
    if (!current || !confirm(`"${current.template ?? "요약"}" 요약을 지울까요?`)) return;
    await api.deleteSummary(recordingId, current.id);
    setShown(null);
    await onSummarized();
  };

  const installed = providers?.filter((p) => p.path) ?? [];

  return (
    <section className="summary">
      <div className="summary-controls">
        <select
          aria-label="요약 템플릿"
          value={template ?? ""}
          disabled={running || templates.length === 0}
          onChange={(e) => setTemplate(e.target.value)}
        >
          {templates.map((t) => (
            <option key={t.id} value={t.id}>
              {t.name}
            </option>
          ))}
        </select>
        <select
          aria-label="요약에 쓸 서비스"
          value={provider ?? ""}
          disabled={running || installed.length === 0}
          onChange={(e) => setProvider(e.target.value as Provider)}
        >
          {providers === null && <option>CLI 찾는 중</option>}
          {providers?.map((p) => (
            <option key={p.provider} value={p.provider} disabled={!p.path}>
              {PROVIDER_LABEL[p.provider]}
              {p.path ? "" : " (설치 안 됨)"}
            </option>
          ))}
        </select>
        {running ? (
          <button onClick={() => api.cancelSummary(recordingId)}>취소</button>
        ) : (
          <button className="primary" onClick={run} disabled={!provider}>
            {summaries.length > 0 ? "새 요약 만들기" : "요약 만들기"}
          </button>
        )}
      </div>
      <p className="hint muted">
        요약은 고른 서비스로 전사 텍스트와 메모만 보냅니다. 녹음 파일은 보내지 않습니다. 이 컴퓨터에 로그인된 구독 계정을 쓰고, API 키는 필요 없습니다. 템플릿은 설정에서 고치고 더할 수 있습니다.
      </p>
      {providers !== null && installed.length === 0 && (
        <p className="error">
          claude 또는 codex CLI가 없습니다. 둘 중 하나를 설치하고 터미널에서 한 번 로그인하면 여기서 쓸 수 있습니다.
        </p>
      )}
      {running && <p className="muted">요약하는 중입니다. 1시간 회의 기준으로 1분 안팎 걸립니다.</p>}
      {error && !running && <div className="error">{error}</div>}
      {summaries.length > 1 && (
        <div className="chips summary-list" role="tablist" aria-label="요약 목록">
          {[...summaries].reverse().map((s) => (
            <button
              key={s.id}
              role="tab"
              aria-selected={s.id === current?.id}
              className={`chip ${s.id === current?.id ? "on" : ""}`}
              onClick={() => setShown(s.id)}
            >
              {s.template ?? "요약"} {new Date(s.createdAt).toLocaleDateString("ko-KR", { month: "numeric", day: "numeric" })}
            </button>
          ))}
        </div>
      )}
      {current && (
        <article className="summary-body">
          <div className="summary-actions">
            <button onClick={copy}>{copied ? "복사했습니다" : "이 요약 복사"}</button>
            <button className="ghost" onClick={remove}>
              지우기
            </button>
          </div>
          <Markdown remarkPlugins={[remarkGfm]}>{current.markdown}</Markdown>
          <p className="muted summary-meta">
            {current.template ? `${current.template}, ` : ""}
            {PROVIDER_LABEL[current.provider]}, {new Date(current.createdAt).toLocaleString("ko-KR")}
          </p>
        </article>
      )}
    </section>
  );
}

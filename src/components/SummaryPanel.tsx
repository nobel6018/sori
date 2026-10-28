import { useEffect, useState } from "react";
import Markdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { api, Provider, PROVIDER_LABEL, ProviderStatus, Summary } from "../api";
import { useCommands } from "../shortcuts";

interface Props {
  recordingId: string;
  summary: Summary | null;
  onSummarized: () => Promise<void>;
}

export function SummaryPanel({ recordingId, summary, onSummarized }: Props) {
  const [providers, setProviders] = useState<ProviderStatus[] | null>(null);
  const [provider, setProvider] = useState<Provider | null>(null);
  const [running, setRunning] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);

  useEffect(() => {
    Promise.all([api.summaryProviders(), api.getSettings()]).then(([list, settings]) => {
      setProviders(list);
      const installed = list.filter((p) => p.path).map((p) => p.provider);
      const remembered = settings.summaryProvider;
      setProvider(remembered && installed.includes(remembered) ? remembered : (installed[0] ?? null));
    });
  }, []);

  const run = async () => {
    if (!provider) return;
    setRunning(true);
    setError(null);
    try {
      await api.summarize(recordingId, provider);
      await onSummarized();
    } catch (e) {
      setError(String(e));
    } finally {
      setRunning(false);
    }
  };

  useCommands((command) => {
    if (command === "summarize" && !running && provider) run();
    if (command === "cancel" && running) api.cancelSummary(recordingId);
  });

  const copy = async () => {
    if (!summary) return;
    await navigator.clipboard.writeText(summary.markdown);
    setCopied(true);
    setTimeout(() => setCopied(false), 1500);
  };

  const installed = providers?.filter((p) => p.path) ?? [];

  return (
    <section className="summary">
      <div className="summary-controls">
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
            {summary ? "다시 요약" : "요약 만들기"}
          </button>
        )}
        {summary && !running && <button onClick={copy}>{copied ? "복사했습니다" : "마크다운 복사"}</button>}
      </div>
      <p className="hint muted">
        요약은 고른 서비스로 전사 텍스트만 보냅니다. 녹음 파일은 보내지 않습니다. 이 컴퓨터에 로그인된 구독 계정을 쓰고, API 키는 필요 없습니다.
      </p>
      {providers !== null && installed.length === 0 && (
        <p className="error">
          claude 또는 codex CLI가 없습니다. 둘 중 하나를 설치하고 터미널에서 한 번 로그인하면 여기서 쓸 수 있습니다.
        </p>
      )}
      {running && <p className="muted">요약하는 중입니다. 1시간 회의 기준으로 1분 안팎 걸립니다.</p>}
      {error && !running && <div className="error">{error}</div>}
      {summary && (
        <article className="summary-body">
          <Markdown remarkPlugins={[remarkGfm]}>{summary.markdown}</Markdown>
          <p className="muted summary-meta">
            {PROVIDER_LABEL[summary.provider]}, {new Date(summary.createdAt).toLocaleString("ko-KR")}
          </p>
        </article>
      )}
    </section>
  );
}

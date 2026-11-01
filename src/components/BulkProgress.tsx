import { useEffect, useState } from "react";
import { stopBulkOrganize, useBulkOrganize } from "../bulkOrganize";

// 예전 녹음 AI 정리의 진행 상황. 설정 줄(자세히)과 사이드바 아래(짧게)에 쓴다. @see docs/organize.md

function useElapsed(since: number | null) {
  const [now, setNow] = useState(Date.now());
  useEffect(() => {
    if (since === null) return;
    const t = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(t);
  }, [since]);
  return since === null ? 0 : Math.max(0, Math.round((now - since) / 1000));
}

/** 설정 줄 안: 몇 번째인지, 지금 어느 녹음인지, 닫아도 되는지 */
export function BulkProgress() {
  const bulk = useBulkOrganize();
  const elapsed = useElapsed(bulk.currentStartedAt);
  if (!bulk.running) return null;
  const pct = bulk.total ? Math.round((bulk.done / bulk.total) * 100) : 0;
  return (
    <div className="bulk-progress" role="status">
      <div className="bulk-bar">
        <div style={{ width: `${pct}%` }} />
      </div>
      <p className="small">
        <span className="spin">↻</span> {bulk.done + 1}/{bulk.total}번째 정리 중
        {bulk.current && ` · #${bulk.current.number} ${bulk.current.title}`} · {elapsed}초
      </p>
      <p className="muted small">
        설정 창을 닫아도 계속되고, 사이드바 아래에 진행 상황이 보입니다. 앱을 끄면 지금 녹음에서 멈추고, 이미 끝난 녹음은 그대로 남습니다. 녹음 하나에 30초~1분쯤
        걸립니다.
      </p>
    </div>
  );
}

/** 사이드바 아래: 한 줄과 멈추기 */
export function BulkBadge() {
  const bulk = useBulkOrganize();
  if (!bulk.running) return null;
  return (
    <div className="bulk-badge" role="status" title={bulk.current ? `#${bulk.current.number} ${bulk.current.title}` : undefined}>
      <span className="spin">↻</span>
      <span>
        AI 정리 {bulk.done + 1}/{bulk.total}
      </span>
      <button className="ghost" onClick={stopBulkOrganize} disabled={bulk.stopping}>
        {bulk.stopping ? "멈추는 중" : "멈추기"}
      </button>
    </div>
  );
}

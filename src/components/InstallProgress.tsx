import { formatBytes } from "../api";
import { InstallState } from "../useModelInstall";

function formatEta(seconds: number): string {
  if (!isFinite(seconds) || seconds <= 0) return "";
  if (seconds < 60) return `약 ${Math.ceil(seconds)}초 남음`;
  return `약 ${Math.ceil(seconds / 60)}분 남음`;
}

export function InstallProgress({ state }: { state: InstallState }) {
  const p = state.progress;
  const ratio = p && p.total > 0 ? p.received / p.total : 0;
  return (
    <div className="install-progress" aria-live="polite">
      <div className="progress-label">
        {p ? (
          <>
            {p.file} 받는 중 ({p.index + 1}/{p.count}), {formatBytes(p.received)} / {formatBytes(p.total)}
            {state.speed > 0 && `, ${(state.speed / 1e6).toFixed(1)}MB/s, ${formatEta((p.total - p.received) / state.speed)}`}
          </>
        ) : (
          "연결하는 중"
        )}
      </div>
      <div className="progress-track">
        <div className="progress-bar" style={{ width: `${ratio * 100}%` }} />
      </div>
    </div>
  );
}

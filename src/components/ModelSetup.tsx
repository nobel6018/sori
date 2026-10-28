import { useState } from "react";
import { formatBytes, ModelCatalog } from "../api";
import { useModelInstall } from "../useModelInstall";
import { InstallProgress } from "./InstallProgress";

interface Props {
  catalog: ModelCatalog;
  onInstalled: (catalog: ModelCatalog) => void;
  onLater: () => void;
}

/** 처음 켰을 때 모델이 없으면 보여 주는 설치 안내 */
export function ModelSetup({ catalog, onInstalled, onLater }: Props) {
  const install = useModelInstall();
  const hasWhisper = catalog.whisper.some((w) => w.path);
  const [choice, setChoice] = useState(catalog.whisper[0].id);
  const needDiarization = catalog.segmentation === null || catalog.embedding === null;
  const whisperIds = hasWhisper ? [] : [choice];
  const total =
    (hasWhisper ? 0 : (catalog.whisper.find((w) => w.id === choice)?.sizeBytes ?? 0)) +
    (needDiarization ? catalog.diarizationBytes : 0);

  const run = async () => {
    const next = await install.start(whisperIds);
    if (next) onInstalled(next);
  };

  return (
    <div className="setup">
      <div className="setup-card">
        <h1>모델을 설치해 주세요</h1>
        <p className="muted">
          sori는 받아 적기와 화자 분리를 이 컴퓨터 안에서 합니다. 그러려면 모델 파일을 한 번 받아야 합니다.
        </p>
        {!hasWhisper && (
          <div className="model-options" role="radiogroup" aria-label="받아 적기 모델">
            {catalog.whisper.map((w, i) => (
              <label key={w.id} className={`model-option ${choice === w.id ? "on" : ""}`}>
                <span className="model-option-head">
                  <input
                    type="radio"
                    name="setup-whisper"
                    checked={choice === w.id}
                    disabled={install.running}
                    onChange={() => setChoice(w.id)}
                  />
                  <span className="model-name">{w.label}</span>
                  <span className="muted">{formatBytes(w.sizeBytes)}</span>
                  {i === 0 && <span className="badge badge-done">추천</span>}
                </span>
                <span className="muted model-desc">{w.description}</span>
              </label>
            ))}
          </div>
        )}
        <p className="muted small">
          {hasWhisper
            ? `화자 분리 모델(${formatBytes(total)})을 받습니다. `
            : needDiarization
              ? `화자 분리 모델(${formatBytes(catalog.diarizationBytes)})까지 모두 ${formatBytes(total)}를 받습니다. `
              : `모두 ${formatBytes(total)}를 받습니다. `}
          중간에 끊기거나 앱을 꺼도, 다시 시작하면 받은 부분부터 이어 받습니다.
        </p>
        {install.running && <InstallProgress state={install} />}
        {install.error && !install.running && <div className="error">{install.error}</div>}
        <div className="setup-actions">
          {install.running ? (
            <button onClick={install.cancel}>멈추기</button>
          ) : (
            <>
              <button className="ghost" onClick={onLater}>
                나중에
              </button>
              <button className="primary" onClick={run}>
                {install.error ? "이어서 받기" : `설치 (${formatBytes(total)})`}
              </button>
            </>
          )}
        </div>
        <p className="muted small">받는 곳: {catalog.modelsDir}</p>
      </div>
    </div>
  );
}

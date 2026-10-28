import { useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { api, formatBytes, InputDevice, installCommand, LANGUAGES, ModelCatalog, Settings } from "../api";
import { useModelInstall } from "../useModelInstall";
import { InstallProgress } from "./InstallProgress";

const MODEL_FIELDS: { key: keyof Settings["models"]; label: string; hint: string }[] = [
  { key: "whisper", label: "Whisper 모델", hint: "ggml 형식 .bin (예: ggml-large-v3-turbo.bin)" },
  { key: "segmentation", label: "화자 구간 모델", hint: "pyannote segmentation 3.0 model.onnx" },
  { key: "embedding", label: "화자 임베딩 모델", hint: "3D-Speaker 등 speaker embedding .onnx" },
];

export function SettingsDialog({ onClose }: { onClose: () => void }) {
  const [settings, setSettings] = useState<Settings | null>(null);
  const [saving, setSaving] = useState(false);
  const [devices, setDevices] = useState<InputDevice[]>([]);
  const [defaultPrompt, setDefaultPrompt] = useState("");
  const [catalog, setCatalog] = useState<ModelCatalog | null>(null);
  const [copied, setCopied] = useState<string | null>(null);
  const refreshCatalog = () => api.modelCatalog().then(setCatalog);
  const install = useModelInstall();
  const [installing, setInstalling] = useState<string | null>(null);

  useEffect(() => {
    api.getSettings().then(setSettings);
    api.defaultSummaryPrompt().then(setDefaultPrompt);
    refreshCatalog();
    api.listInputDevices().then(setDevices).catch(() => setDevices([]));
  }, []);

  if (!settings) return null;

  const pick = async (key: keyof Settings["models"]) => {
    const path = await open({ multiple: false, defaultPath: settings.models[key] });
    if (path) setSettings({ ...settings, models: { ...settings.models, [key]: path } });
  };

  const installWhisper = async (id: string) => {
    setInstalling(id);
    const next = await install.start([id]);
    if (next) {
      setCatalog(next);
      // 받은 모델로 바로 바꾼다. 설치 후 백엔드가 고친 다른 경로도 함께 읽어 온다
      const path = next.whisper.find((w) => w.id === id)?.path;
      const saved = await api.getSettings();
      setSettings({ ...saved, models: { ...saved.models, whisper: path ?? saved.models.whisper } });
    }
    setInstalling(null);
  };

  const copyCommand = async (arg: string) => {
    await navigator.clipboard.writeText(installCommand(arg));
    setCopied(arg);
    setTimeout(() => setCopied(null), 1500);
  };

  const toggleLanguage = (code: string) => {
    const langs = settings.defaultLanguages;
    setSettings({
      ...settings,
      defaultLanguages: langs.includes(code) ? langs.filter((l) => l !== code) : [...langs, code],
    });
  };

  const save = async () => {
    setSaving(true);
    await api.saveSettings(settings);
    setSaving(false);
    onClose();
  };

  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div className="modal" role="dialog" aria-label="설정" onClick={(e) => e.stopPropagation()}>
        <h2>설정</h2>
        <div className="setting-row">
          <div className="setting-label">
            <span>받아 적기 모델</span>
            <button className="ghost" onClick={refreshCatalog}>
              설치 다시 확인
            </button>
          </div>
          <div className="model-options" role="radiogroup" aria-label="Whisper 모델">
            {catalog?.whisper.map((m) => {
              const selected = m.path !== null && m.path === settings.models.whisper;
              return (
                <div key={m.id} className={`model-option ${selected ? "on" : ""}`}>
                  <label>
                    <input
                      type="radio"
                      name="whisper"
                      checked={selected}
                      disabled={m.path === null}
                      onChange={() => m.path && setSettings({ ...settings, models: { ...settings.models, whisper: m.path } })}
                    />
                    <span className="model-name">{m.label}</span>
                    <span className="muted">{formatBytes(m.sizeBytes)}</span>
                    <span className={`badge ${m.path ? "badge-done" : ""}`}>{m.path ? "설치됨" : "설치 안 됨"}</span>
                  </label>
                  <p className="muted">{m.description}</p>
                  {m.path === null && (
                    <div className="install-inline">
                      {installing === m.id && install.running ? (
                        <>
                          <InstallProgress state={install} />
                          <button onClick={install.cancel}>멈추기</button>
                        </>
                      ) : (
                        <button className="primary" disabled={install.running} onClick={() => installWhisper(m.id)}>
                          {installing === m.id && install.error ? "이어서 받기" : `설치 (${formatBytes(m.sizeBytes)})`}
                        </button>
                      )}
                      {installing === m.id && install.error && !install.running && <div className="error">{install.error}</div>}
                      <details>
                        <summary className="muted">터미널로 받기</summary>
                        <div className="install-row">
                          <code>{installCommand(m.scriptArg)}</code>
                          <button onClick={() => copyCommand(m.scriptArg)}>{copied === m.scriptArg ? "복사했습니다" : "명령 복사"}</button>
                        </div>
                      </details>
                    </div>
                  )}
                </div>
              );
            })}
          </div>
          {catalog && (
            <p className="muted small">
              설치하면 없는 화자 분리 모델도 함께 받습니다. 받는 곳: {catalog.modelsDir}
            </p>
          )}
        </div>
        {MODEL_FIELDS.map((f) => (
          <div key={f.key} className="setting-row">
            <div className="setting-label">
              <span>{f.label}</span>
              <span className="muted">{f.hint}</span>
            </div>
            <div className="path-row">
              <input
                value={settings.models[f.key]}
                onChange={(e) => setSettings({ ...settings, models: { ...settings.models, [f.key]: e.target.value } })}
              />
              <button onClick={() => pick(f.key)}>찾기</button>
            </div>
          </div>
        ))}
        <div className="setting-row">
          <div className="setting-label">
            <span>마이크</span>
            <span className="muted">녹음에 쓸 입력 장치</span>
          </div>
          <select
            value={settings.inputDeviceId ?? ""}
            onChange={(e) => setSettings({ ...settings, inputDeviceId: e.target.value || null })}
          >
            <option value="">시스템 기본값{devices.find((d) => d.isDefault) ? ` (${devices.find((d) => d.isDefault)!.name})` : ""}</option>
            {devices.map((d) => (
              <option key={d.id} value={d.id}>
                {d.name}
              </option>
            ))}
          </select>
        </div>
        <label className="setting-toggle">
          <input
            type="checkbox"
            checked={settings.captureSystemAudio}
            onChange={(e) => setSettings({ ...settings, captureSystemAudio: e.target.checked })}
          />
          <span>
            시스템 소리도 함께 녹음
            <span className="muted"> 화상 회의, 통화 상대 목소리처럼 이 컴퓨터에서 나오는 소리. macOS는 14.6 이상, 처음 녹음할 때 권한을 묻습니다</span>
          </span>
        </label>
        <div className="setting-row">
          <div className="setting-label">
            <span>기본 녹음 언어</span>
            <span className="muted">새 녹음을 처리할 때 미리 골라 둡니다</span>
          </div>
          <div className="chips">
            {LANGUAGES.map((l) => (
              <button
                key={l.code}
                className={`chip ${settings.defaultLanguages.includes(l.code) ? "on" : ""}`}
                aria-pressed={settings.defaultLanguages.includes(l.code)}
                onClick={() => toggleLanguage(l.code)}
              >
                {l.label}
              </button>
            ))}
          </div>
        </div>
        <div className="setting-row">
          <div className="setting-label">
            <span>요약 지시문</span>
            <button
              className="ghost"
              disabled={settings.summaryPrompt === null}
              onClick={() => setSettings({ ...settings, summaryPrompt: null })}
            >
              기본값으로
            </button>
          </div>
          <textarea
            className="prompt-input"
            rows={8}
            value={settings.summaryPrompt ?? defaultPrompt}
            onChange={(e) =>
              setSettings({ ...settings, summaryPrompt: e.target.value === defaultPrompt ? null : e.target.value })
            }
          />
        </div>
        <div className="modal-actions">
          <button onClick={onClose}>닫기</button>
          <button className="primary" onClick={save} disabled={saving}>
            저장
          </button>
        </div>
      </div>
    </div>
  );
}

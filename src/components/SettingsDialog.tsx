import { useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { api, InputDevice, LANGUAGES, Settings } from "../api";

const MODEL_FIELDS: { key: keyof Settings["models"]; label: string; hint: string }[] = [
  { key: "whisper", label: "Whisper 모델", hint: "ggml 형식 .bin (예: ggml-large-v3-turbo.bin)" },
  { key: "segmentation", label: "화자 구간 모델", hint: "pyannote segmentation 3.0 model.onnx" },
  { key: "embedding", label: "화자 임베딩 모델", hint: "3D-Speaker 등 speaker embedding .onnx" },
];

export function SettingsDialog({ onClose }: { onClose: () => void }) {
  const [settings, setSettings] = useState<Settings | null>(null);
  const [saving, setSaving] = useState(false);
  const [devices, setDevices] = useState<InputDevice[]>([]);

  useEffect(() => {
    api.getSettings().then(setSettings);
    api.listInputDevices().then(setDevices).catch(() => setDevices([]));
  }, []);

  if (!settings) return null;

  const pick = async (key: keyof Settings["models"]) => {
    const path = await open({ multiple: false, defaultPath: settings.models[key] });
    if (path) setSettings({ ...settings, models: { ...settings.models, [key]: path } });
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

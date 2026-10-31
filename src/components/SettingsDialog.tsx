import { useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { api, formatBytes, GlobalShortcuts, InputDevice, installCommand, LANGUAGES, ModelCatalog, Settings, VoiceInfo } from "../api";
import { acceleratorFromEvent, acceleratorLabel } from "../shortcuts";
import { useModelInstall } from "../useModelInstall";
import { InstallProgress } from "./InstallProgress";
import { SyncSettings } from "./SyncSettings";

const MODEL_FIELDS: { key: keyof Settings["models"]; label: string; hint: string }[] = [
  { key: "whisper", label: "Whisper 모델", hint: "ggml 형식 .bin (예: ggml-large-v3-turbo.bin)" },
  { key: "segmentation", label: "화자 구간 모델", hint: "pyannote segmentation 3.0 model.onnx" },
  { key: "embedding", label: "화자 임베딩 모델", hint: "3D-Speaker 등 speaker embedding .onnx" },
];

export function SettingsDialog({ onClose }: { onClose: () => void }) {
  const [settings, setSettings] = useState<Settings | null>(null);
  const [saving, setSaving] = useState(false);
  const [devices, setDevices] = useState<InputDevice[]>([]);
  const [catalog, setCatalog] = useState<ModelCatalog | null>(null);
  const [copied, setCopied] = useState<string | null>(null);
  const refreshCatalog = () => api.modelCatalog().then(setCatalog);
  const install = useModelInstall();
  const [installing, setInstalling] = useState<string | null>(null);
  const [saveError, setSaveError] = useState<string | null>(null);

  useEffect(() => {
    api.getSettings().then(setSettings);
    refreshCatalog();
    api.listInputDevices().then(setDevices).catch(() => setDevices([]));
  }, []);

  // 단축키를 받는 도중 창을 닫으면 받기 칸의 blur가 오지 않을 수 있다. 꺼 둔 전역 단축키를 다시 켠다
  useEffect(
    () => () => {
      api.resumeShortcuts().catch(() => {});
    },
    [],
  );

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

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
    setSaveError(null);
    try {
      await api.saveSettings(settings);
      onClose();
    } catch (e) {
      // 다른 앱이 쓰는 단축키면 등록이 실패한다. 창을 닫지 않고 고칠 수 있게 둔다
      setSaveError(String(e));
    } finally {
      setSaving(false);
    }
  };

  const setShortcut = (key: keyof GlobalShortcuts, value: string) =>
    setSettings({ ...settings, shortcuts: { ...settings.shortcuts, [key]: value } });

  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div className="modal" role="dialog" aria-label="설정" onClick={(e) => e.stopPropagation()}>
        <h2>설정</h2>
        <SyncSettings value={settings.sync} onChange={(sync) => setSettings({ ...settings, sync })} />
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
        <label className="setting-toggle">
          <input
            type="checkbox"
            checked={settings.defaultDiarize}
            onChange={(e) => setSettings({ ...settings, defaultDiarize: e.target.checked })}
          />
          <span>
            기본으로 화자도 나누기
            <span className="muted"> 끄면 자동 처리와 새 녹음이 전사만 합니다. 혼자 말한 메모나 강의가 많다면 끄세요. 녹음마다 바꿀 수 있습니다</span>
          </span>
        </label>
        <label className="setting-toggle">
          <input type="checkbox" checked={settings.autoProcess} onChange={(e) => setSettings({ ...settings, autoProcess: e.target.checked })} />
          <span>
            녹음을 멈추거나 파일을 가져오면 바로 받아 적기
            <span className="muted"> 화자 수는 자동으로 추정하고 언어는 위의 기본값을 씁니다</span>
          </span>
        </label>
        <label className="setting-toggle">
          <input
            type="checkbox"
            checked={settings.autoSummarize}
            onChange={(e) => setSettings({ ...settings, autoSummarize: e.target.checked })}
          />
          <span>
            받아 적기가 끝나면 기본 템플릿으로 요약
            <span className="muted"> 구독한 claude, codex CLI의 사용량을 씁니다</span>
          </span>
        </label>
        <label className="setting-toggle">
          <input
            type="checkbox"
            checked={settings.meetingDetection}
            onChange={(e) => setSettings({ ...settings, meetingDetection: e.target.checked })}
          />
          <span>
            회의 앱이 마이크를 쓰기 시작하면 녹음할지 묻기
            <span className="muted"> Zoom, Teams, Meet(브라우저), Slack 허들, Discord, FaceTime, 카카오톡 통화. 그 질문으로 시작한 녹음은 통화가 끝나면 멈출지도 묻습니다</span>
          </span>
        </label>
        <label className="setting-toggle">
          <input type="checkbox" checked={settings.autoTag} onChange={(e) => setSettings({ ...settings, autoTag: e.target.checked })} />
          <span>
            받아 적기가 끝나면 AI로 태그 붙이기
            <span className="muted"> 요약과 같은 CLI를 씁니다. 쓰던 태그를 먼저 고르고, 손으로 고친 태그는 덮어쓰지 않습니다. iPhone에서 온 녹음도 받아 적은 뒤 분류합니다</span>
          </span>
        </label>
        <BulkTag />
        <label className="setting-toggle">
          <input
            type="checkbox"
            checked={settings.recordLocation}
            onChange={(e) => setSettings({ ...settings, recordLocation: e.target.checked })}
          />
          <span>
            녹음할 때 위치 남기기
            <span className="muted"> 녹음을 시작할 때 Wi-Fi로 위치를 한 번 받아 주소와 함께 남깁니다. 처음에 macOS가 허락을 묻습니다. iPhone은 iPhone 앱 설정에서 따로 정합니다</span>
          </span>
        </label>
        <TemplateEditor settings={settings} onChange={setSettings} />
        <VoiceList
          enabled={settings.rememberVoices}
          onToggle={(on) => setSettings({ ...settings, rememberVoices: on })}
        />
        <div className="setting-row">
          <div className="setting-label">
            <span>전역 단축키</span>
            <span className="muted">sori가 앞에 없어도 동작합니다. 칸을 누르고 새 조합을 누르세요</span>
          </div>
          <div className="shortcut-settings">
            <ShortcutInput label="녹음 시작/멈춤" value={settings.shortcuts.toggleRecording} onChange={(v) => setShortcut("toggleRecording", v)} />
            <ShortcutInput label="sori 창 앞으로" value={settings.shortcuts.showWindow} onChange={(v) => setShortcut("showWindow", v)} />
            <ShortcutInput label="북마크 (녹음 중)" value={settings.shortcuts.bookmark} onChange={(v) => setShortcut("bookmark", v)} />
          </div>
        </div>
        {saveError && <div className="error">{saveError}</div>}
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

/** 누른 키 조합을 받아 적는 칸. Backspace는 단축키를 끄고, Esc는 받기를 그만둔다 */
function ShortcutInput({ label, value, onChange }: { label: string; value: string; onChange: (v: string) => void }) {
  const [listening, setListening] = useState(false);
  return (
    <label className="shortcut-input">
      <span>{label}</span>
      <button
        className={`shortcut-capture ${listening ? "listening" : ""}`}
        onFocus={() => {
          setListening(true);
          api.suspendShortcuts();
        }}
        onBlur={() => {
          setListening(false);
          // 받기를 마치면 저장된 단축키를 다시 켠다. 새 조합은 저장할 때 등록한다
          api.resumeShortcuts().catch(() => {});
        }}
        onKeyDown={(e) => {
          if (e.key === "Tab") return;
          e.preventDefault();
          // 설정 창의 Esc 닫기까지 가지 않게 여기서 멈춘다
          e.stopPropagation();
          if (e.key === "Escape") return e.currentTarget.blur();
          if (e.key === "Backspace" || e.key === "Delete") {
            onChange("");
            return e.currentTarget.blur();
          }
          const accelerator = acceleratorFromEvent(e.nativeEvent);
          if (accelerator) {
            onChange(accelerator);
            e.currentTarget.blur();
          }
        }}
      >
        {listening ? "키 조합을 누르세요" : <kbd>{acceleratorLabel(value)}</kbd>}
      </button>
    </label>
  );
}

/** 요약 템플릿 고치기. 저장을 눌러야 반영된다 */
function TemplateEditor({ settings, onChange }: { settings: Settings; onChange: (s: Settings) => void }) {
  const templates = settings.summaryTemplates;
  const [selected, setSelected] = useState(templates[0]?.id ?? "");
  const current = templates.find((t) => t.id === selected) ?? templates[0];

  const update = (patch: Partial<{ name: string; prompt: string }>) =>
    onChange({ ...settings, summaryTemplates: templates.map((t) => (t.id === current.id ? { ...t, ...patch } : t)) });

  const add = () => {
    const id = `custom-${Date.now()}`;
    const base = current?.prompt ?? "";
    onChange({ ...settings, summaryTemplates: [...templates, { id, name: "새 템플릿", prompt: base }] });
    setSelected(id);
  };

  const remove = () => {
    if (templates.length <= 1 || !confirm(`"${current.name}" 템플릿을 지울까요?`)) return;
    const rest = templates.filter((t) => t.id !== current.id);
    onChange({
      ...settings,
      summaryTemplates: rest,
      defaultTemplate: settings.defaultTemplate === current.id ? rest[0].id : settings.defaultTemplate,
    });
    setSelected(rest[0].id);
  };

  if (!current) return null;
  return (
    <div className="setting-row">
      <div className="setting-label">
        <span>요약 템플릿</span>
        <button className="ghost" onClick={add}>
          템플릿 더하기
        </button>
      </div>
      <div className="chips">
        {templates.map((t) => (
          <button key={t.id} className={`chip ${t.id === current.id ? "on" : ""}`} onClick={() => setSelected(t.id)}>
            {t.name}
            {t.id === settings.defaultTemplate ? " (기본)" : ""}
          </button>
        ))}
      </div>
      <div className="template-edit">
        <input value={current.name} onChange={(e) => update({ name: e.target.value })} aria-label="템플릿 이름" />
        <button className="ghost" disabled={current.id === settings.defaultTemplate} onClick={() => onChange({ ...settings, defaultTemplate: current.id })}>
          기본으로
        </button>
        <button className="ghost" disabled={templates.length <= 1} onClick={remove}>
          지우기
        </button>
      </div>
      <textarea className="prompt-input" rows={8} value={current.prompt} onChange={(e) => update({ prompt: e.target.value })} aria-label="요약 지시문" />
    </div>
  );
}

/** 기억한 목소리. 이름 바꾸기와 지우기는 바로 반영된다 */
function VoiceList({ enabled, onToggle }: { enabled: boolean; onToggle: (on: boolean) => void }) {
  const [voices, setVoices] = useState<VoiceInfo[]>([]);
  const refresh = () => api.listVoices().then(setVoices);
  useEffect(() => {
    refresh();
  }, []);

  return (
    <div className="setting-row">
      <label className="setting-toggle">
        <input type="checkbox" checked={enabled} onChange={(e) => onToggle(e.target.checked)} />
        <span>
          화자 목소리 기억하기
          <span className="muted">
            {" "}
            화자 이름을 바꾸면 그 목소리를 기억하고, 다음 녹음에서 같은 사람에게 이름을 붙입니다. 목소리 특징은 이 컴퓨터에만 저장됩니다
          </span>
        </span>
      </label>
      {voices.length > 0 && (
        <ul className="voice-list">
          {voices.map((v) => (
            <li key={v.id}>
              <input
                defaultValue={v.name}
                aria-label="목소리 이름"
                onBlur={(e) => {
                  const name = e.target.value.trim();
                  if (name && name !== v.name) api.renameVoice(v.id, name).then(refresh);
                }}
                onKeyDown={(e) => e.key === "Enter" && e.currentTarget.blur()}
              />
              <span className="muted">녹음 {v.samples}개</span>
              <button
                className="ghost"
                onClick={() => confirm(`"${v.name}"의 목소리를 잊을까요?`) && api.deleteVoice(v.id).then(refresh)}
              >
                잊기
              </button>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

/** 아직 태그가 없는 예전 녹음을 AI로 하나씩 분류한다. 한꺼번에 돌리면 구독 사용량 한도에 금방 걸린다
 * @see docs/organize.md */
function BulkTag() {
  const [state, setState] = useState<{ done: number; total: number; failed: number } | null>(null);
  const [running, setRunning] = useState(false);
  const [message, setMessage] = useState<string | null>(null);

  const run = async () => {
    const targets = (await api.listRecordings()).filter((r) => r.status === "done" && !r.tags?.length && !r.tagsEditedAt && !r.autoTaggedAt);
    if (targets.length === 0) {
      setMessage("분류할 녹음이 없습니다. 받아 적기가 끝났고 태그가 없는 녹음만 분류합니다");
      return;
    }
    if (!confirm(`태그 없는 녹음 ${targets.length}개를 AI로 분류할까요? 하나씩 차례로 하고, 구독 사용량을 씁니다.`)) return;
    setRunning(true);
    setMessage(null);
    let failed = 0;
    for (let i = 0; i < targets.length; i++) {
      setState({ done: i, total: targets.length, failed });
      try {
        await api.autoTag(targets[i].id);
      } catch (e) {
        failed += 1;
        console.warn(`분류 실패 ${targets[i].title}: ${e}`);
      }
    }
    setState({ done: targets.length, total: targets.length, failed });
    setRunning(false);
    setMessage(failed ? `${targets.length - failed}개를 분류했고 ${failed}개는 실패했습니다` : `${targets.length}개를 분류했습니다`);
  };

  return (
    <div className="setting-row bulk-tag">
      <button onClick={run} disabled={running}>
        {running && state ? `분류 중 ${state.done}/${state.total}` : "태그 없는 녹음 AI로 분류"}
      </button>
      {message && <span className="muted">{message}</span>}
    </div>
  );
}

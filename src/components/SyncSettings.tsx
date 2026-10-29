import { useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { api, SyncProvider, SyncSettings as SyncSettingsValue, SyncStatus } from "../api";
import { IS_MAC } from "../shortcuts";

const PROVIDERS: { id: SyncProvider; label: string; hint: string }[] = [
  { id: "local", label: "이 기기에만", hint: "동기화하지 않습니다" },
  { id: "icloud", label: "iCloud Drive", hint: IS_MAC ? "iCloud Drive의 sori 폴더" : "iCloud for Windows의 iCloud Drive 폴더" },
  { id: "onedrive", label: "OneDrive", hint: "OneDrive 앱이 동기화하는 폴더의 sori 폴더" },
];

/** 설정 화면의 동기화 항목. 고른 값은 설정 저장과 함께 반영된다 @see docs/sync.md */
export function SyncSettings({ value, onChange }: { value: SyncSettingsValue; onChange: (v: SyncSettingsValue) => void }) {
  const [status, setStatus] = useState<SyncStatus | null>(null);
  const [defaultFolder, setDefaultFolder] = useState<string | null>(null);

  useEffect(() => {
    if (value.provider === "local") return;
    api.defaultSyncFolder(value.provider).then(setDefaultFolder);
  }, [value.provider]);

  useEffect(() => {
    api.syncStatus().then(setStatus);
    const unlisten = api.onSyncStatus(setStatus);
    return () => {
      unlisten.then((f) => f());
    };
  }, []);

  const pickFolder = async () => {
    const dir = await open({ directory: true, defaultPath: value.folder ?? defaultFolder ?? undefined });
    if (typeof dir === "string") onChange({ ...value, folder: dir });
  };

  return (
    <div className="setting-row">
      <div className="setting-label">
        <span>동기화</span>
        <span className="muted">휴대폰 앱, 다른 컴퓨터와 녹음, 전사, 요약, 메모를 나눕니다</span>
      </div>
      <div className="segmented" role="radiogroup" aria-label="동기화 방식">
        {PROVIDERS.map((p) => (
          <button
            key={p.id}
            role="radio"
            aria-checked={value.provider === p.id}
            className={value.provider === p.id ? "on" : ""}
            title={p.hint}
            // 고른 폴더는 클라우드마다 다르므로 바꾸면 새 클라우드의 기본 폴더로 돌아간다
            onClick={() => p.id !== value.provider && onChange({ provider: p.id, folder: null })}
          >
            {p.label}
          </button>
        ))}
      </div>
      {value.provider !== "local" && (
        <div className="path-row">
          <input
            readOnly
            value={value.folder ?? defaultFolder ?? ""}
            placeholder="폴더를 찾지 못했습니다. 바꾸기로 골라 주세요"
            aria-label="동기화 폴더"
          />
          <button onClick={pickFolder}>바꾸기</button>
        </div>
      )}
      {value.provider !== "local" && (
        <p className="muted small">
          처음 켜면 이 기기의 녹음을 모두 올립니다. WAV 오디오는 {IS_MAC ? "AAC(1시간 약 22MB)" : "FLAC(1시간 약 50MB)"}으로 압축해
          올리고, 다른 기기에서 온 녹음의 오디오는 재생하거나 받아 적을 때 받습니다.
        </p>
      )}
      {status && status.provider !== "local" && <SyncStatusLine status={status} />}
    </div>
  );
}

export function SyncStatusLine({ status }: { status: SyncStatus }) {
  return (
    <div className="sync-status-line">
      <span className={status.error ? "error" : "muted"}>{describe(status)}</span>
      <button className="ghost" disabled={status.running} onClick={() => api.syncNow()}>
        지금 동기화
      </button>
    </div>
  );
}

export function describe(status: SyncStatus): string {
  if (status.running) return status.message ?? "동기화 중";
  if (status.error) return `동기화 오류: ${status.error}`;
  if (status.lastSync) return `동기화됨 ${new Date(status.lastSync).toLocaleTimeString("ko-KR", { hour: "2-digit", minute: "2-digit" })}`;
  return "동기화 대기 중";
}

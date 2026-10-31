// iPhone 앱 백엔드 명령(src-tauri/src/lib.rs). 녹음 형식은 데스크톱과 같아 공용 타입을 쓴다
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { Note, Recording, Summary, Transcript } from "../../src/api";

export type { Note, Recording, Summary, Transcript };

/** 목록 한 줄: meta에 전사와 오디오가 이 기기에 있는지를 더한 값 */
export type RecordingItem = Recording & { hasTranscript: boolean; audioAvailable: boolean; hasPreview?: boolean };

export interface RecordingDetail {
  meta: Recording;
  transcript: Transcript | null;
  /** 오래된 것부터 */
  summaries: Summary[];
  audioPath: string;
  audioAvailable: boolean;
  /** Mac 전사가 아직 없어 iPhone 미리 전사를 보여 준다(화자 없음) */
  transcriptPreview: boolean;
}

export interface RecorderState {
  id: string | null;
  recording: boolean;
  time: number;
  level: number;
}

/** 이 기기의 설정 */
export interface Preferences {
  /** 녹음을 시작할 때 위치를 남긴다 */
  recordLocation: boolean;
  /** 녹음이 끝나면 iPhone에서 바로 미리 전사한다(iOS 26 이상) */
  previewTranscribe: boolean;
}

export type SyncProvider = "local" | "icloud";

export interface SyncStatus {
  provider: SyncProvider;
  folder: string | null;
  running: boolean;
  lastSync: string | null;
  error: string | null;
  message: string | null;
}

export const api = {
  listRecordings: () => invoke<RecordingItem[]>("list_recordings"),
  getRecording: (id: string) => invoke<RecordingDetail>("get_recording", { id }),
  renameRecording: (id: string, title: string) => invoke<Recording>("rename_recording", { id, title }),
  setTags: (id: string, tags: string[]) => invoke<Recording>("set_tags", { id, tags }),
  openUrl: (url: string) => invoke<void>("open_url", { url }),
  previewTranscribe: (id: string) => invoke<boolean>("preview_transcribe", { id }),
  getPreferences: () => invoke<Preferences>("get_preferences"),
  setPreferences: (preferences: Preferences) => invoke<void>("set_preferences", { preferences }),
  deleteRecording: (id: string) => invoke<void>("delete_recording", { id }),
  addNote: (id: string, time: number | null, text: string) => invoke<Recording>("add_note", { id, time, text }),
  deleteNote: (id: string, noteId: string) => invoke<Recording>("delete_note", { id, noteId }),
  startRecording: () => invoke<Recording>("start_recording"),
  stopRecording: () => invoke<Recording>("stop_recording"),
  recorderState: () => invoke<RecorderState>("recorder_state"),
  syncStatus: () => invoke<SyncStatus>("sync_status"),
  syncNow: () => invoke<void>("sync_now"),
  chooseIcloudFolder: () => invoke<boolean>("choose_icloud_folder"),
  useLocalOnly: () => invoke<void>("use_local_only"),
  downloadAudio: (id: string) => invoke<void>("download_audio", { id }),
  onSyncStatus: (h: (s: SyncStatus) => void) => listen<SyncStatus>("sync-status", (e) => h(e.payload)),
  onRecordingsSynced: (h: () => void) => listen("recordings-synced", () => h()),
  /** 녹음이 시작되거나 멈췄다. 화면 밖(동작 버튼, 단축어, Live Activity 멈춤 버튼)에서 바뀐 것도 온다 */
  onRecorderChanged: (h: () => void) => listen("recorder-changed", () => h()),
  onRecordingChanged: (h: (id: string) => void) => listen<{ id: string }>("recording-changed", (e) => h(e.payload.id)),
  onAudioDownload: (h: (p: { id: string; done: number; total: number }) => void) =>
    listen<{ id: string; done: number; total: number }>("audio-download", (e) => h(e.payload)),
};

export function syncLabel(s: SyncStatus): string {
  if (s.running) return s.message ?? "동기화 중";
  if (s.error) return "동기화 오류";
  if (s.lastSync) return `동기화됨 ${new Date(s.lastSync).toLocaleTimeString("ko-KR", { hour: "2-digit", minute: "2-digit" })}`;
  return "동기화 대기";
}

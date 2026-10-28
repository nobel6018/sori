import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

export type Status = "recording" | "new" | "processing" | "done" | "failed";
export type Stage = "decoding" | "diarizing" | "transcribing";

export interface Recording {
  id: string;
  title: string;
  createdAt: string;
  sourceName: string;
  audioFile: string;
  status: Status;
  error: string | null;
  duration: number | null;
  speakerNames: Record<string, string>;
  numSpeakers: number | null;
  languages: string[];
  tracks: string[];
}

export interface SpeakerTurn {
  start: number;
  end: number;
  speaker: number;
}

export interface Utterance {
  start: number;
  end: number;
  speaker: number;
  text: string;
  lang: string;
}

export interface Transcript {
  duration: number;
  turns: SpeakerTurn[];
  utterances: Utterance[];
  /** 화자 번호 → 마이크 앞(local) 또는 화상 회의 상대(remote). 트랙을 따로 녹음했을 때만 있다 */
  origins: Record<string, "local" | "remote">;
}

export interface RecordingDetail {
  recording: Recording;
  audioPath: string;
  transcript: Transcript | null;
  summary: Summary | null;
}

export interface Settings {
  models: { whisper: string; segmentation: string; embedding: string };
  defaultLanguages: string[];
  inputDeviceId: string | null;
  captureSystemAudio: boolean;
  summaryProvider: Provider | null;
  summaryPrompt: string | null;
  shortcuts: GlobalShortcuts;
}

/** Tauri 단축키 표기(CommandOrControl+Alt+Shift+R). 빈 문자열이면 끈다 */
export interface GlobalShortcuts {
  toggleRecording: string;
  showWindow: string;
}

/** 전역 단축키와 sori:// 링크로 들어온 요청 */
export type AppAction =
  | { type: "toggleRecording" }
  | { type: "startRecording" }
  | { type: "stopRecording" }
  | { type: "showWindow" }
  | { type: "open"; id: string; time: number | null }
  | { type: "import"; paths: string[]; process: boolean }
  | { type: "search"; query: string };

export interface SearchHit {
  kind: "summary" | "utterance";
  start: number | null;
  speaker: number | null;
  text: string;
}

export interface RecordingHits {
  recording: Recording;
  titleMatch: boolean;
  hits: SearchHit[];
  total: number;
}

export type Provider = "claude" | "codex";

export interface ProviderStatus {
  provider: Provider;
  path: string | null;
}

export interface Summary {
  provider: Provider;
  createdAt: string;
  markdown: string;
}

export const PROVIDER_LABEL: Record<Provider, string> = {
  claude: "Claude (claude CLI)",
  codex: "ChatGPT (codex CLI)",
};

export interface WhisperOption {
  id: string;
  label: string;
  description: string;
  sizeBytes: number;
  scriptArg: string;
  path: string | null;
}

export interface ModelCatalog {
  modelsDir: string;
  whisper: WhisperOption[];
  segmentation: string | null;
  embedding: string | null;
  diarizationBytes: number;
}

export interface DownloadProgress {
  file: string;
  index: number;
  count: number;
  received: number;
  total: number;
}

/** 받아 적기와 화자 분리를 바로 할 수 있는지 */
export function modelsReady(c: ModelCatalog): boolean {
  return c.whisper.some((w) => w.path) && c.segmentation !== null && c.embedding !== null;
}

export function formatBytes(n: number): string {
  return n >= 1e9 ? `${(n / 1e9).toFixed(1)}GB` : `${Math.round(n / 1e6)}MB`;
}

const REPO_RAW = "https://raw.githubusercontent.com/nobel6018/sori/main/scripts";

/** 이 OS에서 모델 설치 스크립트를 한 줄로 실행하는 명령 */
export function installCommand(scriptArg: string): string {
  if (navigator.userAgent.includes("Windows")) {
    return `& ([scriptblock]::Create((irm ${REPO_RAW}/install-models.ps1))) -Whisper ${scriptArg}`;
  }
  return `curl -fsSL ${REPO_RAW}/install-models.sh | bash -s -- --whisper ${scriptArg}`;
}

export interface InputDevice {
  id: string;
  name: string;
  isDefault: boolean;
}

export interface ProgressEvent {
  id: string;
  stage: Stage;
  progress: number;
}

export const api = {
  getSettings: () => invoke<Settings>("get_settings"),
  saveSettings: (settings: Settings) => invoke<void>("save_settings", { settings }),
  modelCatalog: () => invoke<ModelCatalog>("model_catalog"),
  installModels: (whisper: string[]) => invoke<ModelCatalog>("install_models", { whisper }),
  cancelModelInstall: () => invoke<void>("cancel_model_install"),
  onModelDownload: (handler: (p: DownloadProgress) => void) =>
    listen<DownloadProgress>("model-download", (e) => handler(e.payload)),
  listRecordings: () => invoke<Recording[]>("list_recordings"),
  importRecordings: (paths: string[]) => invoke<Recording[]>("import_recordings", { paths }),
  getRecording: (id: string) => invoke<RecordingDetail>("get_recording", { id }),
  updateRecording: (recording: Recording) => invoke<void>("update_recording", { recording }),
  deleteRecording: (id: string) => invoke<void>("delete_recording", { id }),
  cancelProcessing: (id: string) => invoke<void>("cancel_processing", { id }),
  processRecording: (id: string, numSpeakers: number | null, languages: string[]) =>
    invoke<Transcript>("process_recording", { request: { id, numSpeakers, languages } }),
  listInputDevices: () => invoke<InputDevice[]>("list_input_devices"),
  startRecording: () => invoke<Recording>("start_recording"),
  stopRecording: () => invoke<Recording>("stop_recording"),
  onLevel: (handler: (levels: number[]) => void) =>
    listen<{ id: string; levels: number[] }>("recording-level", (e) => handler(e.payload.levels)),
  summaryProviders: () => invoke<ProviderStatus[]>("summary_providers"),
  defaultSummaryPrompt: () => invoke<string>("default_summary_prompt"),
  summarize: (id: string, provider: Provider) => invoke<Summary>("summarize_recording", { id, provider }),
  cancelSummary: (id: string) => invoke<void>("cancel_summary", { id }),
  onRecordingInterrupted: (handler: () => void) => listen("recording-interrupted", () => handler()),
  frontendReady: () => invoke<{ pending: AppAction[]; shortcutError: string | null }>("frontend_ready"),
  onAction: (handler: (a: AppAction) => void) => listen<AppAction>("app-action", (e) => handler(e.payload)),
  searchRecordings: (query: string) => invoke<RecordingHits[]>("search_recordings", { query }),
  revealAudio: (id: string) => invoke<void>("reveal_audio", { id }),
  onProgress: (handler: (e: ProgressEvent) => void) =>
    listen<ProgressEvent>("progress", (e) => handler(e.payload)),
};

export const AUDIO_EXTENSIONS = ["m4a", "mp3", "wav", "aac", "flac", "ogg", "mp4", "mov", "mkv", "webm", "caf"];

export const LANGUAGES: { code: string; label: string }[] = [
  { code: "ko", label: "한국어" },
  { code: "en", label: "영어" },
  { code: "zh", label: "중국어" },
  { code: "ja", label: "일본어" },
  { code: "es", label: "스페인어" },
  { code: "de", label: "독일어" },
  { code: "fr", label: "프랑스어" },
  { code: "vi", label: "베트남어" },
];

export function formatTime(seconds: number): string {
  const s = Math.max(0, Math.floor(seconds));
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const sec = String(s % 60).padStart(2, "0");
  return h > 0 ? `${h}:${String(m).padStart(2, "0")}:${sec}` : `${m}:${sec}`;
}

export function speakerName(recording: Recording, speaker: number): string {
  return recording.speakerNames[String(speaker)] ?? `화자 ${speaker + 1}`;
}

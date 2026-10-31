import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

export type Status = "recording" | "new" | "processing" | "done" | "failed";
export type Stage = "waiting" | "decoding" | "diarizing" | "transcribing";

export interface Recording {
  id: string;
  /** 사람이 부르기 쉬운 번호(#12). 만든 순서대로 붙는다 */
  number: number;
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
  /** 마지막 처리에서 화자를 나눴는지 */
  diarize: boolean;
  /** 녹음하며 남긴 메모. 글이 비면 북마크 */
  notes: Note[];
  /** 화자 번호 → 목소리로 알아본 기억 id */
  voiceIds: Record<string, string>;
  /** `/`로 계층을 만든 태그(업무/영업). docs/organize.md */
  tags?: string[];
  /** 사람이 태그를 고친 시각. 있으면 AI가 덮어쓰지 않는다 */
  tagsEditedAt?: string;
  /** AI가 태그를 붙인 시각 */
  autoTaggedAt?: string;
  /** 녹음을 시작한 곳 */
  location?: RecordingLocation | null;
  /** 녹음 시각과 겹치는 캘린더 일정 */
  event?: CalendarEvent | null;
  /** 사람이 제목을 고친 시각. 있으면 AI가 제목을 바꾸지 않는다 */
  titleEditedAt?: string;
}

export interface CalendarEvent {
  title: string;
  start: string;
  end: string;
  /** 참석자 이름(나 빼고). 화자 이름 후보 */
  attendees: string[];
  calendar?: string | null;
}

/** AI가 뽑은 할 일 후보. 확인한 것만 미리 알림에 넣는다 */
export interface Todo {
  id: string;
  text: string;
  /** 기한(YYYY-MM-DD) */
  due?: string | null;
  /** 미리 알림에 넣은 시각 */
  addedAt?: string | null;
}

export interface RecordingLocation {
  lat: number;
  lon: number;
  /** 오차 반경(m) */
  accuracy?: number | null;
  /** 주소나 장소 이름 */
  place?: string | null;
  capturedAt: string;
}

export interface Note {
  time: number;
  text: string;
  /** 동기화할 때 붙는다 */
  id?: string;
  /** 메모를 남긴 기기. 없으면 이 기기 */
  device?: string;
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
  /** 오래된 것부터 */
  summaries: Summary[];
  /** 오디오가 이 기기에 있는지. 다른 기기에서 온 녹음은 받기 전까지 없다 */
  audioAvailable: boolean;
}

/** 동기화는 macOS의 iCloud Drive만. Windows는 이 기기에만 저장한다 */
export type SyncProvider = "local" | "icloud";

export interface SyncSettings {
  provider: SyncProvider;
  /** iCloud Drive 안의 sori 폴더. null이면 기본 위치 */
  folder: string | null;
}

export interface SyncStatus {
  provider: SyncProvider;
  running: boolean;
  lastSync: string | null;
  error: string | null;
  message: string | null;
}

export interface SummaryTemplate {
  id: string;
  name: string;
  prompt: string;
}

export interface VoiceInfo {
  id: string;
  name: string;
  samples: number;
  updatedAt: string;
}

export interface Answer {
  markdown: string;
  /** 전체 녹음 질문에서 [R3 12:34]의 R3가 가리키는 녹음 */
  sources: { key: string; recordingId: string; title: string }[];
}

export type MeetingPrompt = { kind: "start"; app: string } | { kind: "stop"; app: string };

export interface Settings {
  models: { whisper: string; segmentation: string; embedding: string };
  defaultLanguages: string[];
  inputDeviceId: string | null;
  captureSystemAudio: boolean;
  summaryProvider: Provider | null;
  summaryPrompt: string | null;
  shortcuts: GlobalShortcuts;
  autoProcess: boolean;
  /** 자동 처리와 새 녹음의 기본 방식. false면 전사만 */
  defaultDiarize: boolean;
  autoSummarize: boolean;
  /** 받아 적기가 끝나면 AI로 제목, 태그, 할 일 후보를 붙인다(docs/organize.md) */
  autoTag: boolean;
  /** AI 정리 때 겹치는 캘린더 일정을 쓴다 */
  useCalendar: boolean;
  /** 녹음을 시작할 때 위치를 남긴다 */
  recordLocation: boolean;
  rememberVoices: boolean;
  meetingDetection: boolean;
  summaryTemplates: SummaryTemplate[];
  defaultTemplate: string | null;
  sync: SyncSettings;
}

/** Tauri 단축키 표기(CommandOrControl+Alt+Shift+R). 빈 문자열이면 끈다 */
export interface GlobalShortcuts {
  toggleRecording: string;
  showWindow: string;
  bookmark: string;
}

/** 전역 단축키와 sori:// 링크로 들어온 요청 */
export type AppAction =
  | { type: "toggleRecording" }
  | { type: "startRecording" }
  | { type: "stopRecording" }
  | { type: "bookmark" }
  | { type: "showWindow" }
  | { type: "open"; id: string; time: number | null }
  | { type: "import"; paths: string[]; process: boolean }
  | { type: "search"; query: string }
  | { type: "blocked"; message: string };

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
  id: string;
  /** 만든 템플릿 이름. 예전 요약은 없다 */
  template: string | null;
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
  setTags: (id: string, tags: string[]) => invoke<Recording>("set_tags", { id, tags }),
  /** AI로 제목, 태그, 할 일 후보를 정리한다. 이미 정리했으면(force가 아니면) null */
  organize: (id: string, force = false) => invoke<Recording | null>("organize_recording", { id, force }),
  getTodos: (id: string) => invoke<Todo[]>("get_todos", { id }),
  saveTodos: (id: string, todos: Todo[]) => invoke<void>("save_todos", { id, todos }),
  addTodosToReminders: (id: string, todoIds: string[]) => invoke<number>("add_todos_to_reminders", { id, todoIds }),
  deleteRecording: (id: string) => invoke<void>("delete_recording", { id }),
  cancelProcessing: (id: string) => invoke<void>("cancel_processing", { id }),
  /** diarize가 null이면 설정의 기본 방식 */
  processRecording: (id: string, numSpeakers: number | null, languages: string[], diarize: boolean | null = null) =>
    invoke<Transcript>("process_recording", { request: { id, numSpeakers, languages, diarize } }),
  listInputDevices: () => invoke<InputDevice[]>("list_input_devices"),
  startRecording: () => invoke<Recording>("start_recording"),
  stopRecording: () => invoke<Recording>("stop_recording"),
  onLevel: (handler: (levels: number[]) => void) =>
    listen<{ id: string; levels: number[] }>("recording-level", (e) => handler(e.payload.levels)),
  summaryProviders: () => invoke<ProviderStatus[]>("summary_providers"),
  defaultSummaryPrompt: () => invoke<string>("default_summary_prompt"),
  summarize: (id: string, provider: Provider | null, template: string | null) =>
    invoke<Summary>("summarize_recording", { id, provider, template }),
  summaryRunning: (id: string) => invoke<boolean>("summary_running", { id }),
  onSummaryState: (handler: (e: { id: string; running: boolean }) => void) =>
    listen<{ id: string; running: boolean }>("summary-state", (e) => handler(e.payload)),
  deleteSummary: (id: string, summaryId: string) => invoke<void>("delete_summary", { id, summaryId }),
  ask: (question: string, recordingId: string | null, provider: Provider) =>
    invoke<Answer>("ask_recordings", { question, recordingId, provider }),
  /** 질문 취소. 요약 취소와 같은 통로를 쓴다 */
  cancelAsk: (recordingId: string | null) => invoke<void>("cancel_summary", { id: recordingId ? `ask:${recordingId}` : "ask" }),
  addNote: (id: string | null, time: number | null, text: string) => invoke<Recording>("add_note", { id, time, text }),
  deleteNote: (id: string, index: number) => invoke<Recording>("delete_note", { id, index }),
  syncStatus: () => invoke<SyncStatus>("sync_status"),
  syncNow: () => invoke<void>("sync_now"),
  /** macOS가 아니면 null */
  defaultSyncFolder: (provider: SyncProvider) => invoke<string | null>("default_sync_folder", { provider }),
  downloadAudio: (id: string) => invoke<void>("download_audio", { id }),
  onSyncStatus: (handler: (s: SyncStatus) => void) => listen<SyncStatus>("sync-status", (e) => handler(e.payload)),
  /** 다른 기기에서 녹음이 오거나 지워졌다 */
  onRecordingsSynced: (handler: () => void) => listen("recordings-synced", () => handler()),
  onAudioDownload: (handler: (p: { id: string; done: number; total: number }) => void) =>
    listen<{ id: string; done: number; total: number }>("audio-download", (e) => handler(e.payload)),
  onRecordingChanged: (handler: (id: string) => void) =>
    listen<{ id: string }>("recording-changed", (e) => handler(e.payload.id)),
  nameSpeaker: (id: string, speaker: number, name: string) => invoke<Recording>("name_speaker", { id, speaker, name }),
  listVoices: () => invoke<VoiceInfo[]>("list_voices"),
  renameVoice: (id: string, name: string) => invoke<void>("rename_voice", { id, name }),
  deleteVoice: (id: string) => invoke<void>("delete_voice", { id }),
  /** 백엔드가 저장 창을 띄우고 고른 곳에 쓴다. 취소하면 false */
  saveExport: (fileName: string, bytes: Uint8Array) =>
    invoke<boolean>("save_export", bytes, { headers: { "x-name": encodeURIComponent(fileName) } }),
  meetingPrompt: () => invoke<MeetingPrompt | null>("meeting_prompt"),
  meetingPromptRespond: (accept: boolean) => invoke<void>("meeting_prompt_respond", { accept }),
  cancelSummary: (id: string) => invoke<void>("cancel_summary", { id }),
  onRecordingInterrupted: (handler: () => void) => listen("recording-interrupted", () => handler()),
  frontendReady: () => invoke<{ pending: AppAction[]; shortcutError: string | null }>("frontend_ready"),
  onAction: (handler: (a: AppAction) => void) => listen<AppAction>("app-action", (e) => handler(e.payload)),
  searchRecordings: (query: string) => invoke<RecordingHits[]>("search_recordings", { query }),
  suspendShortcuts: () => invoke<void>("suspend_shortcuts"),
  resumeShortcuts: () => invoke<void>("resume_shortcuts"),
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

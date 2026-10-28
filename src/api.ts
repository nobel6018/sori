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

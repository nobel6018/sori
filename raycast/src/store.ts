import { execFile } from "node:child_process";
import { readFile, readdir, stat } from "node:fs/promises";
import { homedir } from "node:os";
import { join } from "node:path";

// sori 앱이 저장한 파일을 직접 읽는다. 앱이 꺼져 있어도 검색과 복사가 된다.
// 형식은 src-tauri/src/store.rs, src-tauri/src/engine/mod.rs와 같아야 한다
export const DATA_DIR = join(homedir(), "Library/Application Support/com.nobel6018.sori/recordings");

export interface Recording {
  id: string;
  number?: number;
  title: string;
  createdAt: string;
  status: "recording" | "new" | "processing" | "done" | "failed";
  duration: number | null;
  speakerNames: Record<string, string>;
  notes?: { time: number; text: string }[];
}

export interface Utterance {
  start: number;
  end: number;
  speaker: number;
  text: string;
}

export interface Transcript {
  utterances: Utterance[];
  origins?: Record<string, "local" | "remote">;
}

export interface Summary {
  markdown: string;
  createdAt: string;
}

export interface Detail {
  recording: Recording;
  transcript: Transcript | null;
  summary: Summary | null;
}

async function readJson<T>(path: string): Promise<T | null> {
  try {
    return JSON.parse(await readFile(path, "utf8")) as T;
  } catch {
    return null;
  }
}

export async function listRecordings(): Promise<Recording[]> {
  let names: string[];
  try {
    names = await readdir(DATA_DIR);
  } catch {
    return [];
  }
  const all = await Promise.all(names.map((n) => readJson<Recording>(join(DATA_DIR, n, "meta.json"))));
  return all.filter((r): r is Recording => r !== null).sort((a, b) => b.createdAt.localeCompare(a.createdAt));
}

/** 전사는 1시간 회의면 수백 KB라 수정 시각이 같으면 다시 읽지 않는다 */
const transcriptCache = new Map<string, { mtime: number; transcript: Transcript }>();

export async function getTranscript(id: string): Promise<Transcript | null> {
  const path = join(DATA_DIR, id, "transcript.json");
  try {
    const { mtimeMs } = await stat(path);
    const cached = transcriptCache.get(id);
    if (cached && cached.mtime === mtimeMs) return cached.transcript;
    const transcript = await readJson<Transcript>(path);
    if (transcript) transcriptCache.set(id, { mtime: mtimeMs, transcript });
    return transcript;
  } catch {
    return null;
  }
}

export function getSummary(id: string): Promise<Summary | null> {
  return readJson<Summary>(join(DATA_DIR, id, "summary.json"));
}

export async function getDetail(recording: Recording): Promise<Detail> {
  const [transcript, summary] = await Promise.all([getTranscript(recording.id), getSummary(recording.id)]);
  return { recording, transcript, summary };
}

export function speakerName(recording: Recording, speaker: number): string {
  return recording.speakerNames?.[String(speaker)] ?? `화자 ${speaker + 1}`;
}

export function formatTime(seconds: number): string {
  const s = Math.max(0, Math.floor(seconds));
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const sec = String(s % 60).padStart(2, "0");
  return h > 0 ? `${h}:${String(m).padStart(2, "0")}:${sec}` : `${m}:${sec}`;
}

/**
 * sori:// 링크를 연다. -g로 열어 녹음 토글처럼 창이 필요 없는 요청에 sori가 앞으로 나오지 않게 한다.
 * 창이 필요한 요청(열기, 가져오기)은 sori가 스스로 창을 앞으로 가져온다
 */
export async function openSori(path: string, params: [string, string][] = []): Promise<void> {
  // 녹음과 가져오기 링크는 sori가 만든 토큰이 있어야 받는다. 웹 페이지가 녹음을 켜지 못하게 하는 장치다
  if (path.startsWith("record/") || path === "import") {
    const token = await readFile(join(DATA_DIR, "..", "link-token"), "utf8").catch(() => null);
    if (!token) throw new Error("sori를 한 번 실행한 뒤 다시 시도하세요");
    params = [...params, ["token", token.trim()]];
  }
  const query = params.map(([k, v]) => `${k}=${encodeURIComponent(v)}`).join("&");
  const url = `sori://${path}${query ? `?${query}` : ""}`;
  return new Promise((resolve, reject) => {
    execFile("open", ["-g", url], (error) => (error ? reject(new Error("sori가 설치돼 있지 않거나 링크를 열지 못했습니다")) : resolve()));
  });
}

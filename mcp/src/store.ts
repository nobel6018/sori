import { readFile, readdir, stat } from "node:fs/promises";
import { homedir } from "node:os";
import { join } from "node:path";

// sori 앱이 저장한 파일을 직접 읽는다. 쓰지는 않는다.
// 형식은 src-tauri/src/store.rs, src-tauri/src/engine/mod.rs와 같아야 한다

function dataDir(): string {
  const id = "com.nobel6018.sori";
  if (process.platform === "darwin") return join(homedir(), "Library/Application Support", id);
  if (process.platform === "win32") return join(process.env.APPDATA ?? join(homedir(), "AppData/Roaming"), id);
  return join(process.env.XDG_DATA_HOME ?? join(homedir(), ".local/share"), id);
}

export const RECORDINGS_DIR = process.env.SORI_DATA_DIR ? join(process.env.SORI_DATA_DIR, "recordings") : join(dataDir(), "recordings");

export interface Note {
  time: number;
  text: string;
}

export interface Recording {
  id: string;
  title: string;
  createdAt: string;
  status: string;
  duration: number | null;
  speakerNames: Record<string, string>;
  notes?: Note[];
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
  template?: string | null;
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
    names = await readdir(RECORDINGS_DIR);
  } catch {
    return [];
  }
  const all = await Promise.all(names.map((n) => readJson<Recording>(join(RECORDINGS_DIR, n, "meta.json"))));
  return all.filter((r): r is Recording => r !== null).sort((a, b) => b.createdAt.localeCompare(a.createdAt));
}

const cache = new Map<string, { mtime: number; transcript: Transcript }>();

export async function getTranscript(id: string): Promise<Transcript | null> {
  const path = join(RECORDINGS_DIR, id, "transcript.json");
  try {
    const { mtimeMs } = await stat(path);
    const hit = cache.get(id);
    if (hit && hit.mtime === mtimeMs) return hit.transcript;
    const t = await readJson<Transcript>(path);
    if (t) cache.set(id, { mtime: mtimeMs, transcript: t });
    return t;
  } catch {
    return null;
  }
}

export function getSummary(id: string): Promise<Summary | null> {
  return readJson<Summary>(join(RECORDINGS_DIR, id, "summary.json"));
}

export function speakerName(r: Recording, speaker: number): string {
  return r.speakerNames?.[String(speaker)] ?? `화자 ${speaker + 1}`;
}

export function formatTime(seconds: number): string {
  const s = Math.max(0, Math.floor(seconds));
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const sec = String(s % 60).padStart(2, "0");
  return h > 0 ? `${h}:${String(m).padStart(2, "0")}:${sec}` : `${m}:${sec}`;
}

/** 대소문자와 띄어쓰기를 무시한다. src-tauri/src/search.rs normalize와 같다 */
export function normalize(s: string): string {
  return s.replace(/\s+/g, "").toLowerCase();
}

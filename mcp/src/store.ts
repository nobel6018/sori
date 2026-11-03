import { execFile } from "node:child_process";
import { readFile, readdir, stat } from "node:fs/promises";
import { homedir } from "node:os";
import { join } from "node:path";

// sori 앱이 저장한 파일을 직접 읽는다. 쓰지는 않는다. 처리와 요약은 sori:// 링크로 앱에 맡긴다.
// 형식은 src-tauri/src/store.rs, src-tauri/src/engine/mod.rs와 같아야 한다

function dataDir(): string {
  const id = "com.nobel6018.sori";
  if (process.platform === "darwin") return join(homedir(), "Library/Application Support", id);
  if (process.platform === "win32") return join(process.env.APPDATA ?? join(homedir(), "AppData/Roaming"), id);
  return join(process.env.XDG_DATA_HOME ?? join(homedir(), ".local/share"), id);
}

export const DATA_DIR = process.env.SORI_DATA_DIR ?? dataDir();
export const RECORDINGS_DIR = join(DATA_DIR, "recordings");

export interface Note {
  time: number;
  text: string;
}

export interface Recording {
  id: string;
  number?: number;
  title: string;
  createdAt: string;
  status: string;
  duration: number | null;
  speakerNames: Record<string, string>;
  notes?: Note[];
  error?: string | null;
  /** `/`로 계층을 만든 태그(업무/영업) */
  tags?: string[];
  sourceName?: string;
}

/** 앱이 받아 적기와 요약 중에 적는 진행 상황. src-tauri/src/activity.rs Entry와 같다 */
export interface Activity {
  stage: "waiting" | "decoding" | "diarizing" | "transcribing" | "summarizing" | string;
  progress: number | null;
  updatedAt: string;
}

/** src-tauri/src/store.rs Todo와 같다 */
export interface Todo {
  id: string;
  text: string;
  due?: string | null;
  addedAt?: string | null;
}

export interface SummaryTemplate {
  id: string;
  name: string;
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

/** 번호("12", "#12")나 id로 녹음을 찾는다 */
export async function findRecording(key: string): Promise<Recording | undefined> {
  const k = key.trim().replace(/^#/, "");
  const all = await listRecordings();
  return /^\d+$/.test(k) ? all.find((x) => x.number === Number(k)) : all.find((x) => x.id === k);
}

export function getRecording(id: string): Promise<Recording | null> {
  return readJson<Recording>(join(RECORDINGS_DIR, id, "meta.json"));
}

/**
 * 녹음, 처리 링크가 함께 보내야 하는 값. sori가 처음 실행할 때 데이터 폴더에 만든다(권한 600).
 * 웹 페이지는 이 파일을 읽지 못하므로, 이 값이 있으면 이 컴퓨터의 사용자가 보낸 요청으로 본다
 * @see ../../docs/shortcuts-and-deep-links.md
 */
export async function linkToken(): Promise<string | null> {
  try {
    const t = (await readFile(join(DATA_DIR, "link-token"), "utf8")).trim();
    return t.length >= 32 ? t : null;
  } catch {
    return null;
  }
}

/** sori:// 링크를 연다. 셸을 거치지 않아 링크 안의 &가 명령으로 해석되지 않는다 */
export function openLink(url: string): Promise<void> {
  const [cmd, args] =
    process.platform === "darwin"
      ? ["open", ["-g", url]] // -g: sori 창을 앞으로 가져오지 않는다
      : process.platform === "win32"
        ? ["rundll32", ["url.dll,FileProtocolHandler", url]]
        : ["xdg-open", [url]];
  return new Promise((resolve, reject) => {
    execFile(cmd, args as string[], (e) => (e ? reject(new Error(`${cmd} 실행 실패: ${e.message}`)) : resolve()));
  });
}

/** 진행 상황 파일. 앱이 비정상 종료해 남은 오래된 파일(10분 넘게 안 바뀜)은 없는 것으로 본다 */
export async function getActivity(id: string): Promise<Activity | null> {
  const a = await readJson<Activity>(join(DATA_DIR, "activity", `${id}.json`));
  if (!a || Date.now() - new Date(a.updatedAt).getTime() > 10 * 60 * 1000) return null;
  return a;
}

export async function getTodos(id: string): Promise<Todo[]> {
  return (await readJson<Todo[]>(join(RECORDINGS_DIR, id, "todos.json"))) ?? [];
}

/** 앱 설정의 요약 템플릿과 기본 템플릿, 자동 요약 여부 */
export async function summarySettings(): Promise<{ templates: SummaryTemplate[]; defaultTemplate: string | null; autoSummarize: boolean }> {
  const s = await readJson<{ summaryTemplates?: SummaryTemplate[]; defaultTemplate?: string; autoSummarize?: boolean }>(
    join(DATA_DIR, "settings.json"),
  );
  // 설정에 템플릿이 없으면 앱은 기본 템플릿을 쓴다. 회의록(meeting)이 첫 번째다(src-tauri/src/summarize.rs default_templates)
  const templates = s?.summaryTemplates?.length ? s.summaryTemplates : [{ id: "meeting", name: "회의록" }];
  return {
    templates: templates.map((t) => ({ id: t.id, name: t.name })),
    defaultTemplate: s?.defaultTemplate ?? templates[0]?.id ?? null,
    autoSummarize: s?.autoSummarize === true,
  };
}

/** "1:02:03", "12:30", "75" 같은 시각을 초로. 못 읽으면 null */
export function parseTime(v: string | number | undefined): number | null {
  if (v === undefined) return null;
  if (typeof v === "number") return v >= 0 ? v : null;
  const parts = v.trim().split(":").map(Number);
  if (parts.length === 0 || parts.length > 3 || parts.some((n) => !Number.isFinite(n) || n < 0)) return null;
  return parts.reduce((acc, n) => acc * 60 + n, 0);
}

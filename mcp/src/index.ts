#!/usr/bin/env node
import { stat } from "node:fs/promises";
import { basename, extname, isAbsolute, join } from "node:path";
import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";
import { z } from "zod";
import {
  Activity,
  findRecording,
  formatTime,
  getActivity,
  getRecording,
  getSummary,
  getTodos,
  getTranscript,
  linkToken,
  listRecordings,
  normalize,
  openLink,
  parseTime,
  Recording,
  RECORDINGS_DIR,
  speakerName,
  summarySettings,
  Transcript,
} from "./store.js";

// sori 녹음을 MCP 클라이언트(Claude Code, Claude 데스크톱 등)에서 찾고 읽는다.
// 받아 적기, 요약, 화자 이름, 미리 알림, 가져오기처럼 앱 데이터를 바꾸는 일은 직접 하지 않고
// 토큰을 붙인 sori:// 링크로 앱에 맡긴다(엔진과 설정이 앱에 있고, 파일을 앱과 동시에 쓰면 충돌한다)
// @see ../README.md

const server = new McpServer({ name: "sori", version: "0.7.0" });

const text = (s: string) => ({ content: [{ type: "text" as const, text: s }] });
const fail = (s: string) => ({ ...text(s), isError: true });
const notFound = (id: string) => fail(`${id} 녹음이 없습니다. list_recordings로 번호를 확인해 주세요.`);
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
const label = (r: Recording) => `#${r.number ?? "?"} ${r.title}`;

const ID = z.string().min(1).describe('녹음 번호("12" 또는 "#12")나 id');

const STATUS: Record<string, string> = {
  recording: "녹음 중",
  new: "아직 처리 안 함",
  processing: "처리 중",
  done: "처리 끝",
  failed: "처리 실패",
};

const STAGE: Record<string, string> = {
  waiting: "앞 녹음 처리를 기다리는 중",
  decoding: "오디오 읽는 중",
  diarizing: "화자 나누는 중",
  transcribing: "받아 적는 중",
  summarizing: "요약하는 중",
};

const AUDIO_EXTENSIONS = ["m4a", "mp3", "wav", "aac", "flac", "ogg", "mp4", "mov", "mkv", "webm", "caf"];

function activityText(a: Activity | null): string | null {
  if (!a) return null;
  const pct = a.progress != null ? ` ${Math.round(a.progress * 100)}%` : "";
  return `${STAGE[a.stage] ?? a.stage}${pct}`;
}

function dateOf(r: Recording): string {
  return new Date(r.createdAt).toLocaleString("ko-KR", { dateStyle: "medium", timeStyle: "short" });
}

async function statusLine(r: Recording): Promise<string> {
  const now = activityText(await getActivity(r.id));
  const base = STATUS[r.status] ?? r.status;
  if (now) return r.status === "processing" ? now : `${base}, ${now}`;
  return r.status === "failed" && r.error ? `${base} (${r.error})` : base;
}

/**
 * 한 화자가 이어서 말한 발화는 한 줄로 묶는다. 화자를 골라 읽을 때는 사이에 다른 사람이 말한 곳에서 줄을 나눈다
 * (안 나누면 그 사람의 말이 회의 전체에 걸쳐 한 줄로 붙는다)
 */
function transcriptLines(r: Recording, t: Transcript, filter: { from: number; to: number; speakers: Set<number> | null }) {
  const lines: string[] = [];
  let previous = -2;
  t.utterances.forEach((u, i) => {
    if (u.end < filter.from || u.start > filter.to) return;
    if (filter.speakers && !filter.speakers.has(u.speaker)) return;
    if (previous === i - 1 && t.utterances[previous].speaker === u.speaker) lines[lines.length - 1] += ` ${u.text.trim()}`;
    else lines.push(`[${formatTime(u.start)}] ${speakerName(r, u.speaker)}: ${u.text.trim()}`);
    previous = i;
  });
  return lines;
}

/** 화자별 발화 시간. 화자 이름을 붙일 때 누가 누구인지 가늠하는 데 쓴다 */
function speakerTable(r: Recording, t: Transcript): string[] {
  const time = new Map<number, number>();
  for (const u of t.utterances) time.set(u.speaker, (time.get(u.speaker) ?? 0) + (u.end - u.start));
  return [...time.entries()]
    .sort((a, b) => a[0] - b[0])
    .map(([s, sec]) => {
      const named = r.speakerNames?.[String(s)];
      return `- 화자 ${s + 1}${named ? `: ${named}` : ""} (${(sec / 60).toFixed(1)}분)`;
    });
}

/** "영훈", "화자 2", "2" 를 화자 번호(0부터)로 */
function speakerIndex(r: Recording, key: string): number | null {
  const k = key.trim();
  const m = k.match(/^(?:화자\s*)?(\d+)$/);
  if (m) return Number(m[1]) - 1;
  const hit = Object.entries(r.speakerNames ?? {}).find(([, name]) => normalize(name) === normalize(k));
  return hit ? Number(hit[0]) : null;
}

/** check가 참이 될 때까지 2초마다 본다. 기다리는 동안 클라이언트가 원하면 진행 알림을 보낸다 */
async function waitUntil(
  check: () => Promise<boolean>,
  seconds: number,
  progress?: { report: (message: string) => Promise<void>; message: () => Promise<string | null> },
): Promise<boolean> {
  const until = Date.now() + seconds * 1000;
  while (Date.now() < until) {
    if (await check()) return true;
    if (progress) {
      const m = await progress.message();
      if (m) await progress.report(m).catch(() => {});
    }
    await sleep(2000);
  }
  return check();
}

type Extra = {
  _meta?: { progressToken?: string | number };
  sendNotification: (n: { method: "notifications/progress"; params: Record<string, unknown> }) => Promise<void>;
};

/** 클라이언트가 progressToken을 보냈을 때만 진행 알림을 보낸다 */
function reporter(extra: Extra) {
  const token = extra._meta?.progressToken;
  if (token === undefined) return undefined;
  let n = 0;
  return async (message: string) => {
    n++;
    await extra.sendNotification({ method: "notifications/progress", params: { progressToken: token, progress: n, message } });
  };
}

async function transcriptTime(id: string): Promise<number> {
  try {
    return (await stat(join(RECORDINGS_DIR, id, "transcript.json"))).mtimeMs;
  } catch {
    return 0;
  }
}

async function summaryTime(id: string): Promise<string | null> {
  return (await getSummary(id))?.createdAt ?? null;
}

const NO_TOKEN =
  "sori 링크 토큰이 없습니다. sori 앱을 한 번 실행하면 데이터 폴더에 link-token 파일이 생깁니다. 이 컴퓨터에 sori 앱이 설치돼 있어야 합니다";
const NOT_RECEIVED = (version: string) =>
  `sori가 요청을 받지 않았습니다. sori 앱이 ${version} 이상인지(이전 버전은 이 링크를 모릅니다), 앱에 오류 알림이 떴는지 확인해 주세요`;

/** 토큰을 붙여 sori:// 링크를 연다. 토큰은 결과 글에 넣지 않는다 */
async function send(path: string, q: URLSearchParams): Promise<string | null> {
  const token = await linkToken();
  if (!token) return NO_TOKEN;
  q.set("token", token);
  await openLink(`sori://${path}?${q}`);
  return null;
}

/** 요약 템플릿 id를 확인한다. 틀리면 고를 수 있는 목록을 알려 준다 */
async function checkTemplate(template: string | undefined): Promise<string | null> {
  if (!template) return null;
  const { templates } = await summarySettings();
  if (templates.some((t) => t.id === template)) return null;
  return `요약 템플릿 "${template}"이 없습니다. 고를 수 있는 템플릿: ${templates.map((t) => `${t.id}(${t.name})`).join(", ")}`;
}

/* ───────── 찾기와 읽기 ───────── */

server.registerTool(
  "list_recordings",
  {
    title: "sori 녹음 목록",
    description:
      "sori에 있는 녹음을 최신순으로 보여 준다. 번호(#12), 제목, id, 날짜, 길이, 상태, 태그, 요약 여부를 돌려준다. " +
      "날짜 범위, 태그, 상태로 거를 수 있다. 사용자는 녹음을 번호로 부른다",
    inputSchema: {
      limit: z.number().int().min(1).max(200).default(20).describe("최대 개수"),
      since: z.string().regex(/^\d{4}-\d{2}-\d{2}$/).optional().describe("이 날짜(YYYY-MM-DD, 이 컴퓨터 시간대)부터"),
      until: z.string().regex(/^\d{4}-\d{2}-\d{2}$/).optional().describe("이 날짜(YYYY-MM-DD)까지, 그날 포함"),
      tag: z.string().min(1).optional().describe("태그. 상위 태그를 주면 하위 태그도 포함(업무 → 업무/영업)"),
      status: z.enum(["recording", "new", "processing", "done", "failed"]).optional().describe("처리 상태"),
    },
    annotations: { readOnlyHint: true },
  },
  async ({ limit, since, until, tag, status }) => {
    const from = since ? new Date(`${since}T00:00:00`).getTime() : -Infinity;
    const to = until ? new Date(`${until}T23:59:59.999`).getTime() : Infinity;
    const wanted = tag ? normalize(tag).replace(/\/+$/, "") : null;
    const all = (await listRecordings())
      .filter((r) => {
        const t = new Date(r.createdAt).getTime();
        if (t < from || t > to) return false;
        if (status && r.status !== status) return false;
        if (wanted && !(r.tags ?? []).some((x) => normalize(x) === wanted || normalize(x).startsWith(`${wanted}/`))) return false;
        return true;
      })
      .slice(0, limit);
    const rows = await Promise.all(
      all.map(async (r) => {
        const summary = await getSummary(r.id);
        const length = r.duration != null ? formatTime(r.duration) : "길이 모름";
        const tags = r.tags?.length ? `, 태그 ${r.tags.join(" ")}` : "";
        return `- ${label(r)} (id: ${r.id}, ${dateOf(r)}, ${length}, ${await statusLine(r)}${tags}${summary ? ", 요약 있음" : ""})`;
      }),
    );
    return text(rows.length ? rows.join("\n") : "조건에 맞는 녹음이 없습니다.");
  },
);

server.registerTool(
  "search_recordings",
  {
    title: "sori 전사 검색",
    description:
      "모든 녹음의 제목, 요약, 전사에서 낱말을 찾는다. 띄어쓰기와 대소문자는 무시한다. 찾은 문장과 녹음 id, 시각을 돌려준다. 자세한 내용은 get_recording으로 읽는다",
    inputSchema: {
      query: z.string().min(1).describe("찾을 낱말이나 구절"),
      limit: z.number().int().min(1).max(200).default(40).describe("최대 결과 수"),
    },
    annotations: { readOnlyHint: true },
  },
  async ({ query, limit }) => {
    const needle = normalize(query);
    const asNumber = /^#?\d+$/.test(needle) ? Number(needle.replace("#", "")) : null;
    const out: string[] = [];
    for (const r of await listRecordings()) {
      if (out.length >= limit) break;
      const hits: string[] = [];
      if (r.number === asNumber) hits.push("  - (번호)");
      if (normalize(r.title).includes(needle)) hits.push("  - (제목)");
      const summary = await getSummary(r.id);
      for (const line of summary?.markdown.split("\n") ?? []) {
        if (normalize(line).includes(needle)) hits.push(`  - (요약) ${line.trim()}`);
      }
      const t = await getTranscript(r.id);
      for (const u of t?.utterances ?? []) {
        if (normalize(u.text).includes(needle)) hits.push(`  - [${formatTime(u.start)}] ${speakerName(r, u.speaker)}: ${u.text.trim()}`);
      }
      if (hits.length) {
        out.push(`- ${label(r)} (id: ${r.id}, ${dateOf(r)})`, ...hits.slice(0, Math.max(1, limit - out.length)));
      }
    }
    return text(out.length ? out.join("\n") : `"${query}"를 찾지 못했습니다.`);
  },
);

server.registerTool(
  "get_recording",
  {
    title: "sori 녹음 읽기",
    description:
      "녹음 하나의 정보(처리 상태와 진행률 포함), 화자별 발화 시간, 최근 요약, 메모, 전사를 마크다운으로 돌려준다. " +
      "긴 회의는 from, to(시각), speaker(화자)로 전사 일부만 읽고, max_chars로 길이를 줄인다",
    inputSchema: {
      id: ID,
      transcript: z.boolean().default(true).describe("전사를 함께 돌려줄지"),
      summary: z.boolean().default(true).describe("요약을 함께 돌려줄지"),
      from: z.union([z.string(), z.number()]).optional().describe('전사 시작 시각. "12:30", "1:02:03", 초'),
      to: z.union([z.string(), z.number()]).optional().describe("전사 끝 시각"),
      speakers: z.array(z.string()).optional().describe('이 화자만. 이름이나 번호("화자 2", "2")'),
      max_chars: z.number().int().min(500).max(400000).default(60000).describe("전사 최대 글자 수. 넘으면 뒤를 자른다"),
    },
    annotations: { readOnlyHint: true },
  },
  async ({ id, transcript, summary, from, to, speakers, max_chars }) => {
    const r = await findRecording(id);
    if (!r) return notFound(id);
    const lines = [`# ${r.title}`, "", `- 번호: #${r.number ?? "?"}`, `- id: ${r.id}`, `- 날짜: ${dateOf(r)}`];
    if (r.duration != null) lines.push(`- 길이: ${formatTime(r.duration)}`);
    lines.push(`- 상태: ${await statusLine(r)}`);
    if (r.tags?.length) lines.push(`- 태그: ${r.tags.join(", ")}`);
    const t = await getTranscript(r.id);
    if (!t && r.status === "new") lines.push("- 전사가 없습니다. process_recording으로 sori에 받아 적기를 맡길 수 있습니다");
    if (t) lines.push("", "## 화자", "", ...speakerTable(r, t));

    const s = summary ? await getSummary(r.id) : null;
    if (s) lines.push("", `## 요약${s.template ? ` (${s.template})` : ""}`, "", s.markdown.trim());
    if (r.notes?.length) lines.push("", "## 메모", "", ...r.notes.map((n) => `- [${formatTime(n.time)}] ${n.text || "(북마크)"}`));

    if (transcript) {
      if (!t) lines.push("", "## 전사", "", "아직 받아 적지 않았습니다.");
      else {
        const start = parseTime(from) ?? 0;
        const end = parseTime(to) ?? Infinity;
        if ((from !== undefined && parseTime(from) === null) || (to !== undefined && parseTime(to) === null)) {
          return fail('시각을 읽지 못했습니다. "12:30", "1:02:03", 초(숫자)로 주세요.');
        }
        let only: Set<number> | null = null;
        if (speakers?.length) {
          const idx = speakers.map((k) => speakerIndex(r, k));
          const bad = speakers.filter((_, i) => idx[i] === null);
          if (bad.length) return fail(`화자를 찾지 못했습니다: ${bad.join(", ")}. "화자 2"처럼 번호로 주거나 화자 목록의 이름을 써 주세요.`);
          only = new Set(idx as number[]);
        }
        let body = transcriptLines(r, t, { from: start, to: end, speakers: only }).join("\n");
        const range = from !== undefined || to !== undefined || only ? " (일부)" : "";
        if (body.length > max_chars) {
          // 자른 다음 위치를 알려 주어 이어 읽을 수 있게 한다
          // 줄 경계에서 자른다. 첫 줄부터 한도보다 길면 그 줄 중간에서 자른다
          const nl = body.lastIndexOf("\n", max_chars);
          const cut = nl > 0 ? nl : max_chars;
          const rest = body.slice(cut).replace(/^\n/, "");
          const next = rest.match(/^\[([\d:]+)\]/)?.[1];
          body = `${body.slice(0, cut)}\n\n(전사가 길어 여기서 잘랐습니다.${next ? ` from: "${next}"로 이어 읽을 수 있습니다.` : ""})`;
        }
        lines.push("", `## 전사${range}`, "", body || "이 범위에 말한 내용이 없습니다.");
      }
    }
    return text(lines.join("\n"));
  },
);

server.registerTool(
  "get_todos",
  {
    title: "sori 할 일",
    description: "요약에서 AI가 뽑은 할 일 후보를 돌려준다(할 일 id, 내용, 기한, 미리 알림에 넣었는지). 미리 알림에 넣으려면 add_todos_to_reminders",
    inputSchema: { id: ID },
    annotations: { readOnlyHint: true },
  },
  async ({ id }) => {
    const r = await findRecording(id);
    if (!r) return notFound(id);
    const todos = await getTodos(r.id);
    if (!todos.length) return text(`${label(r)}에 할 일 후보가 없습니다. 요약이 없거나, 요약에 할 일이 없었을 수 있습니다.`);
    return text(
      [
        `${label(r)} 할 일 ${todos.length}개`,
        ...todos.map((t) => `- ${t.text}${t.due ? ` (기한 ${t.due})` : ""}${t.addedAt ? " [미리 알림에 넣음]" : ""} (할 일 id: ${t.id})`),
      ].join("\n"),
    );
  },
);

/* ───────── 앱에 맡기는 일 ───────── */

server.registerTool(
  "process_recording",
  {
    title: "sori 녹음 받아 적기",
    description:
      "sori 앱에 녹음의 받아 적기(전사, 화자 분리)를 맡기고, 원하면 요약까지 이어서 시킨다. 앱이 꺼져 있으면 켜진다. " +
      "이미 전사가 있으면 새로 덮어쓴다. 1시간 녹음은 몇 분 걸린다. wait_seconds를 주면 그만큼 끝나길 기다리며 진행률을 알리고, " +
      "0이면 시작만 확인하고 돌아온다. 진행률과 상태는 get_recording으로도 볼 수 있다. 화자 수를 알면 speakers로 넣는다(자동 추정보다 정확하다)",
    inputSchema: {
      id: ID,
      speakers: z.number().int().min(1).max(20).optional().describe("화자 수. 모르면 비워 둔다(앱이 추정)"),
      languages: z
        .array(z.string().regex(/^[a-z]{2,5}$/))
        .optional()
        .describe('whisper 언어 코드. 예: ["ko"], 섞였으면 ["ko", "en"]. 비우면 앱 설정'),
      diarize: z.boolean().optional().describe("화자를 나눌지. 비우면 앱 설정"),
      summarize: z.boolean().optional().describe("받아 적은 뒤 요약까지 할지. 비우면 앱 설정의 자동 요약"),
      template: z.string().optional().describe("요약 템플릿 id(meeting, one-on-one 등). 비우면 앱의 기본 템플릿"),
      wait_seconds: z.number().int().min(0).max(900).default(0).describe("끝날 때까지 기다릴 최대 시간(초)"),
    },
    annotations: { readOnlyHint: false, destructiveHint: true, idempotentHint: false, openWorldHint: false },
  },
  async ({ id, speakers, languages, diarize, summarize, template, wait_seconds }, extra) => {
    const r = await findRecording(id);
    if (!r) return notFound(id);
    if (r.status === "recording") return fail(`${label(r)}은 아직 녹음 중입니다. 녹음을 멈춘 뒤 다시 요청해 주세요.`);
    if (r.status === "processing") return fail(`${label(r)}은 이미 처리 중입니다(${await statusLine(r)}).`);
    const badTemplate = await checkTemplate(template);
    if (badTemplate) return fail(badTemplate);

    const q = new URLSearchParams({ id: r.id });
    if (speakers) q.set("speakers", String(speakers));
    for (const l of languages ?? []) q.append("lang", l);
    if (diarize !== undefined) q.set("diarize", diarize ? "1" : "0");
    if (summarize !== undefined) q.set("summarize", summarize ? "1" : "0");
    if (template) q.set("template", template);
    const before = { transcript: await transcriptTime(r.id), summary: await summaryTime(r.id) };
    const error = await send("process", q);
    if (error) return fail(error);

    // 앱이 받으면 곧바로 상태를 "처리 중"으로 바꾼다. 앱을 새로 켜는 경우를 생각해 넉넉히 기다린다
    const started = await waitUntil(async () => {
      const m = await getRecording(r.id);
      return m?.status === "processing" || (await transcriptTime(r.id)) > before.transcript;
    }, 45);
    if (!started) return fail(NOT_RECEIVED("0.10.0"));

    const options = [
      speakers ? `화자 ${speakers}명` : "화자 수 자동",
      languages?.length ? `언어 ${languages.join(", ")}` : "언어는 앱 설정",
    ].join(", ");
    const wantSummary = summarize ?? (await summarySettings()).autoSummarize;
    if (wait_seconds === 0) {
      return text(
        `${label(r)} 받아 적기를 시작했습니다(${options}${wantSummary ? ", 끝나면 요약" : ""}). ` +
          `길이 ${r.duration != null ? formatTime(r.duration) : "모름"}. 진행률은 get_recording으로 볼 수 있습니다.`,
      );
    }

    const t0 = Date.now();
    const report = reporter(extra as unknown as Extra);
    const finished = await waitUntil(
      async () => {
        const m = await getRecording(r.id);
        if (m?.status === "failed") return true;
        if (m?.status !== "done") return false;
        return !wantSummary || (await summaryTime(r.id)) !== before.summary;
      },
      wait_seconds,
      report && { report, message: async () => activityText(await getActivity(r.id)) },
    );
    const m = await getRecording(r.id);
    const took = formatTime((Date.now() - t0) / 1000);
    if (m?.status === "failed") return fail(`${label(r)} 처리에 실패했습니다: ${m.error ?? "이유 모름"}`);
    if (!finished) {
      const now = activityText(await getActivity(r.id)) ?? (m?.status === "done" ? "요약 기다리는 중" : "처리 중");
      return text(`${label(r)} 아직 끝나지 않았습니다(${now}, ${took} 기다림). 나중에 get_recording으로 확인해 주세요.`);
    }
    return text(`${label(r)} ${wantSummary ? "받아 적기와 요약을" : "받아 적기를"} 마쳤습니다(${took}). get_recording으로 읽을 수 있습니다.`);
  },
);

server.registerTool(
  "summarize_recording",
  {
    title: "sori 녹음 요약",
    description:
      "이미 받아 적은 녹음을 sori 앱에 다시 요약시킨다. 요약 방식(Claude, Codex)은 앱 설정을 따른다. " +
      "template으로 템플릿을 고를 수 있다(회의록, 1:1 면담, 인터뷰, 강의, 할 일만 등. 틀린 id를 주면 목록을 알려 준다). " +
      "전사가 없으면 process_recording에 summarize: true로 요청한다. wait_seconds를 주면 그만큼 끝나길 기다린다",
    inputSchema: {
      id: ID,
      template: z.string().optional().describe("요약 템플릿 id. 비우면 앱의 기본 템플릿"),
      wait_seconds: z.number().int().min(0).max(600).default(0).describe("끝날 때까지 기다릴 최대 시간(초)"),
    },
    annotations: { readOnlyHint: false, destructiveHint: false, idempotentHint: false, openWorldHint: false },
  },
  async ({ id, template, wait_seconds }) => {
    const r = await findRecording(id);
    if (!r) return notFound(id);
    if (r.status === "processing") return fail(`${label(r)}은 받아 적는 중입니다. 끝난 뒤 요청해 주세요.`);
    if (!(await getTranscript(r.id))) {
      return fail(`${label(r)}은 아직 전사가 없습니다. process_recording에 summarize: true로 요청해 주세요.`);
    }
    if ((await getActivity(r.id))?.stage === "summarizing") return fail(`${label(r)}은 이미 요약하는 중입니다.`);
    const badTemplate = await checkTemplate(template);
    if (badTemplate) return fail(badTemplate);
    const before = await summaryTime(r.id);
    const q = new URLSearchParams({ id: r.id });
    if (template) q.set("template", template);
    const error = await send("summarize", q);
    if (error) return fail(error);
    if (wait_seconds === 0) return text(`${label(r)} 요약을 요청했습니다. 끝나면 get_recording에서 볼 수 있습니다.`);
    const done = await waitUntil(async () => (await summaryTime(r.id)) !== before, wait_seconds);
    return done
      ? text(`${label(r)} 요약을 마쳤습니다. get_recording으로 읽을 수 있습니다.`)
      : text(`${label(r)} 아직 요약 중입니다. 앱에 오류 알림이 떴다면 요약 설정(Claude, Codex 로그인)을 확인해 주세요.`);
  },
);

server.registerTool(
  "set_speaker_names",
  {
    title: "sori 화자 이름 붙이기",
    description:
      '전사의 "화자 1, 화자 2"에 이름을 붙인다. 앱에서 이름을 바꾸는 것과 같아, 목소리 기억이 켜져 있으면 다음 녹음에서도 알아본다. ' +
      "대화 속 호칭으로 누가 누구인지 추정했다면 확신이 있을 때만 붙이고, 사용자에게 추정이라고 알린다. 빈 이름은 이름을 지운다",
    inputSchema: {
      id: ID,
      names: z
        .array(
          z.object({
            speaker: z.number().int().min(1).max(99).describe('전사와 get_recording 화자 목록의 번호("화자 2"면 2)'),
            name: z.string().max(40).describe("붙일 이름. 빈 문자열이면 지운다"),
          }),
        )
        .min(1),
    },
    annotations: { readOnlyHint: false, destructiveHint: false, idempotentHint: true, openWorldHint: false },
  },
  async ({ id, names }) => {
    const r = await findRecording(id);
    if (!r) return notFound(id);
    const t = await getTranscript(r.id);
    if (!t) return fail(`${label(r)}은 아직 전사가 없습니다.`);
    const present = new Set(t.utterances.map((u) => u.speaker));
    const missing = names.filter((n) => !present.has(n.speaker - 1)).map((n) => n.speaker);
    if (missing.length) return fail(`전사에 없는 화자 번호입니다: ${missing.join(", ")}. get_recording의 화자 목록을 확인해 주세요.`);

    const q = new URLSearchParams({ id: r.id });
    for (const n of names) q.append("name", `${n.speaker - 1}:${n.name.trim()}`);
    const error = await send("speakers", q);
    if (error) return fail(error);
    const want = (m: Recording | null) =>
      names.every((n) => (m?.speakerNames?.[String(n.speaker - 1)] ?? "") === n.name.trim());
    const ok = await waitUntil(async () => want(await getRecording(r.id)), 30);
    if (!ok) return fail(NOT_RECEIVED("0.11.0"));
    return text(`${label(r)} 화자 이름을 붙였습니다: ${names.map((n) => `화자 ${n.speaker} → ${n.name.trim() || "(지움)"}`).join(", ")}`);
  },
);

server.registerTool(
  "set_title",
  {
    title: "sori 녹음 제목 바꾸기",
    description:
      "녹음 제목을 바꾼다. 앱에서 제목을 고치는 것과 같아, 이후 AI 정리가 이 제목을 다시 바꾸지 않는다. " +
      "제목을 지어 붙일 때는 get_recording으로 내용을 보고 짓는다",
    inputSchema: {
      id: ID,
      title: z.string().trim().min(1).max(120).describe("새 제목"),
    },
    annotations: { readOnlyHint: false, destructiveHint: false, idempotentHint: true, openWorldHint: false },
  },
  async ({ id, title }) => {
    const r = await findRecording(id);
    if (!r) return notFound(id);
    const error = await send("title", new URLSearchParams({ id: r.id, title }));
    if (error) return fail(error);
    const ok = await waitUntil(async () => (await getRecording(r.id))?.title === title, 30);
    if (!ok) return fail(NOT_RECEIVED("0.13.0"));
    return text(`#${r.number ?? "?"} 제목을 바꿨습니다: ${r.title} → ${title}`);
  },
);

server.registerTool(
  "add_todos_to_reminders",
  {
    title: "sori 할 일을 미리 알림에",
    description:
      "get_todos로 본 할 일 중 고른 것을 macOS 미리 알림 앱에 넣는다. 이미 넣은 것은 다시 넣지 않는다. " +
      "처음에는 macOS가 미리 알림 접근을 허락할지 묻는다. 사용자가 고른 할 일만 넣는다",
    inputSchema: {
      id: ID,
      todo_ids: z.array(z.string().min(1).max(64)).min(1).describe("get_todos가 돌려준 할 일 id"),
    },
    annotations: { readOnlyHint: false, destructiveHint: false, idempotentHint: true, openWorldHint: true },
  },
  async ({ id, todo_ids }) => {
    const r = await findRecording(id);
    if (!r) return notFound(id);
    const todos = await getTodos(r.id);
    const unknown = todo_ids.filter((t) => !todos.some((x) => x.id === t));
    if (unknown.length) return fail(`없는 할 일 id입니다: ${unknown.join(", ")}. get_todos로 확인해 주세요.`);
    const fresh = todo_ids.filter((t) => !todos.find((x) => x.id === t)?.addedAt);
    if (!fresh.length) return text("고른 할 일은 모두 이미 미리 알림에 넣었습니다.");

    const q = new URLSearchParams({ id: r.id });
    for (const t of fresh) q.append("todo", t);
    const error = await send("reminders", q);
    if (error) return fail(error);
    // 처음이면 macOS 권한 창이 떠서 사람이 눌러야 한다
    const ok = await waitUntil(async () => {
      const now = await getTodos(r.id);
      return fresh.every((t) => now.find((x) => x.id === t)?.addedAt);
    }, 60);
    if (!ok) {
      return fail(
        "미리 알림에 넣었는지 확인하지 못했습니다. 미리 알림 접근 허락 창이 떴는지, sori 앱에 오류 알림이 떴는지 확인해 주세요(sori 0.11.0 이상 필요).",
      );
    }
    return text(`${label(r)} 할 일 ${fresh.length}개를 미리 알림에 넣었습니다.`);
  },
);

server.registerTool(
  "import_audio",
  {
    title: "sori로 오디오 가져오기",
    description:
      "이 컴퓨터의 오디오, 영상 파일(Teams, Zoom 녹화 등)을 sori로 가져오고, 원하면 바로 받아 적게 한다. 파일은 sori 폴더로 복사된다. " +
      `받는 확장자: ${AUDIO_EXTENSIONS.join(", ")}. 절대 경로만 받는다`,
    inputSchema: {
      paths: z.array(z.string().min(1)).min(1).max(20).describe("가져올 파일의 절대 경로"),
      process: z.boolean().default(true).describe("가져온 뒤 받아 적을지"),
      speakers: z.number().int().min(1).max(20).optional().describe("화자 수(모든 파일에 같이 씀)"),
      languages: z.array(z.string().regex(/^[a-z]{2,5}$/)).optional().describe('언어 코드. 예: ["ko"]'),
      summarize: z.boolean().optional().describe("받아 적은 뒤 요약까지 할지. 비우면 앱 설정"),
      template: z.string().optional().describe("요약 템플릿 id"),
    },
    annotations: { readOnlyHint: false, destructiveHint: false, idempotentHint: false, openWorldHint: false },
  },
  async ({ paths, process: doProcess, speakers, languages, summarize, template }) => {
    const bad: string[] = [];
    for (const p of paths) {
      const ext = extname(p).slice(1).toLowerCase();
      const ok = isAbsolute(p) && AUDIO_EXTENSIONS.includes(ext) && (await stat(p).then((s) => s.isFile()).catch(() => false));
      if (!ok) bad.push(p);
    }
    if (bad.length) return fail(`가져올 수 없는 파일입니다(절대 경로의 오디오, 영상 파일만): ${bad.join(", ")}`);
    const badTemplate = await checkTemplate(template);
    if (badTemplate) return fail(badTemplate);

    const known = new Set((await listRecordings()).map((r) => r.id));
    const q = new URLSearchParams();
    for (const p of paths) q.append("path", p);
    // 화자 수, 언어를 주려면 가져오기와 처리를 나눈다. process=0이면 앱이 설정의 자동 처리도 하지 않는다(0.11.0~)
    q.set("process", "0");
    const error = await send("import", q);
    if (error) return fail(error);

    // 큰 파일은 복사에 시간이 걸린다
    let added: Recording[] = [];
    await waitUntil(async () => {
      added = (await listRecordings()).filter((r) => !known.has(r.id));
      return added.length >= paths.length;
    }, 120);
    if (!added.length) return fail(NOT_RECEIVED("0.10.0"));
    added.sort((a, b) => a.createdAt.localeCompare(b.createdAt));
    const names = added.map((r) => `${label(r)}${r.sourceName ? ` (${basename(r.sourceName)})` : ""}`);
    if (!doProcess) return text(`가져왔습니다:\n${names.map((n) => `- ${n}`).join("\n")}`);

    for (const r of added) {
      const p = new URLSearchParams({ id: r.id });
      if (speakers) p.set("speakers", String(speakers));
      for (const l of languages ?? []) p.append("lang", l);
      if (summarize !== undefined) p.set("summarize", summarize ? "1" : "0");
      if (template) p.set("template", template);
      const e = await send("process", p);
      if (e) return fail(e);
    }
    return text(
      `가져와서 받아 적기를 맡겼습니다(앱이 한 번에 하나씩 처리합니다):\n${names.map((n) => `- ${n}`).join("\n")}\n진행률은 get_recording으로 볼 수 있습니다.`,
    );
  },
);

await server.connect(new StdioServerTransport());

#!/usr/bin/env node
import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";
import { z } from "zod";
import { readFile, stat } from "node:fs/promises";
import { join } from "node:path";
import {
  findRecording,
  formatTime,
  getRecording,
  getSummary,
  getTranscript,
  linkToken,
  listRecordings,
  normalize,
  openLink,
  Recording,
  RECORDINGS_DIR,
  speakerName,
  Transcript,
} from "./store.js";

// sori 녹음을 MCP 클라이언트(Claude Code, Claude 데스크톱 등)에서 찾고 읽는다.
// 받아 적기와 요약은 직접 하지 않고 sori:// 링크로 앱에 맡긴다(엔진과 요약 설정이 앱에 있다)
// @see ../README.md

const server = new McpServer({ name: "sori", version: "0.5.0" });

const STATUS: Record<string, string> = {
  recording: "녹음 중",
  new: "아직 처리 안 함",
  processing: "처리 중",
  done: "처리 끝",
  failed: "처리 실패",
};

const text = (s: string) => ({ content: [{ type: "text" as const, text: s }] });

function dateOf(r: Recording): string {
  return new Date(r.createdAt).toLocaleString("ko-KR", { dateStyle: "medium", timeStyle: "short" });
}

function transcriptText(r: Recording, t: Transcript): string {
  const lines: string[] = [];
  let last: number | null = null;
  for (const u of t.utterances) {
    if (u.speaker === last) lines[lines.length - 1] += ` ${u.text.trim()}`;
    else lines.push(`[${formatTime(u.start)}] ${speakerName(r, u.speaker)}: ${u.text.trim()}`);
    last = u.speaker;
  }
  return lines.join("\n");
}

server.registerTool(
  "list_recordings",
  {
    title: "sori 녹음 목록",
    description: "sori에 있는 녹음을 최신순으로 보여 준다. 번호(#12), 제목, id, 날짜, 길이, 요약 여부를 돌려준다. 사용자는 녹음을 번호로 부른다",
    inputSchema: { limit: z.number().int().min(1).max(200).default(20).describe("최대 개수") },
    annotations: { readOnlyHint: true },
  },
  async ({ limit }) => {
    const all = (await listRecordings()).slice(0, limit);
    const rows = await Promise.all(
      all.map(async (r) => {
        const summary = await getSummary(r.id);
        const length = r.duration != null ? formatTime(r.duration) : "길이 모름";
        return `- #${r.number ?? "?"} ${r.title} (id: ${r.id}, ${dateOf(r)}, ${length}, ${r.status}${summary ? ", 요약 있음" : ""})`;
      }),
    );
    return text(rows.length ? rows.join("\n") : "녹음이 없습니다.");
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
        out.push(`- #${r.number ?? "?"} ${r.title} (id: ${r.id}, ${dateOf(r)})`, ...hits.slice(0, Math.max(1, limit - out.length)));
      }
    }
    return text(out.length ? out.join("\n") : `"${query}"를 찾지 못했습니다.`);
  },
);

server.registerTool(
  "get_recording",
  {
    title: "sori 녹음 읽기",
    description: "녹음 하나의 정보, 최근 요약, 메모, 전사를 마크다운으로 돌려준다. 전사는 1시간 회의면 길므로 필요할 때만 켠다",
    inputSchema: {
      id: z.string().min(1).describe("녹음 번호(\"12\" 또는 \"#12\")나 list_recordings, search_recordings가 돌려준 id"),
      transcript: z.boolean().default(true).describe("전사를 함께 돌려줄지"),
    },
    annotations: { readOnlyHint: true },
  },
  async ({ id, transcript }) => {
    const r = await findRecording(id);
    if (!r) return notFound(id);
    const lines = [`# ${r.title}`, "", `- 번호: #${r.number ?? "?"}`, `- 날짜: ${dateOf(r)}`];
    if (r.duration != null) lines.push(`- 길이: ${formatTime(r.duration)}`);
    lines.push(`- 상태: ${STATUS[r.status] ?? r.status}${r.status === "failed" && r.error ? ` (${r.error})` : ""}`);
    if (r.status === "new") lines.push("- 전사가 없습니다. process_recording으로 sori에 받아 적기를 맡길 수 있습니다");
    const summary = await getSummary(r.id);
    if (summary) lines.push("", `## 요약${summary.template ? ` (${summary.template})` : ""}`, "", summary.markdown.trim());
    if (r.notes?.length) lines.push("", "## 메모", "", ...r.notes.map((n) => `- [${formatTime(n.time)}] ${n.text || "(북마크)"}`));
    if (transcript) {
      const t = await getTranscript(r.id);
      lines.push("", "## 전사", "", t ? transcriptText(r, t) : "아직 받아 적지 않았습니다.");
    }
    return text(lines.join("\n"));
  },
);

const notFound = (id: string) => ({ ...text(`${id} 녹음이 없습니다. list_recordings로 번호를 확인해 주세요.`), isError: true });
const fail = (s: string) => ({ ...text(s), isError: true });
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

/** check가 참이 될 때까지 2초마다 본다. 시간 안에 참이 되면 true */
async function waitUntil(check: () => Promise<boolean>, seconds: number): Promise<boolean> {
  const until = Date.now() + seconds * 1000;
  while (Date.now() < until) {
    if (await check()) return true;
    await sleep(2000);
  }
  return check();
}

async function summaryTime(id: string): Promise<string | null> {
  return (await getSummary(id))?.createdAt ?? null;
}

async function transcriptTime(id: string): Promise<number> {
  try {
    return (await stat(join(RECORDINGS_DIR, id, "transcript.json"))).mtimeMs;
  } catch {
    return 0;
  }
}

/** 앱 설정의 "자동 요약". 처리 링크에 요약 여부를 안 줬을 때 앱이 따르는 값이다 */
async function autoSummarize(): Promise<boolean> {
  try {
    const settings = JSON.parse(await readFile(join(RECORDINGS_DIR, "..", "settings.json"), "utf8"));
    return settings.autoSummarize === true;
  } catch {
    return false;
  }
}

const NO_TOKEN =
  "sori 링크 토큰이 없습니다. sori 앱을 한 번 실행하면 데이터 폴더에 link-token 파일이 생깁니다. 이 컴퓨터에 sori 앱이 설치돼 있어야 합니다";
const NOT_RECEIVED =
  "sori가 요청을 받지 않았습니다. sori 앱이 0.10.0 이상인지(이전 버전은 이 링크를 모릅니다), 앱에 오류 알림이 떴는지 확인해 주세요";

server.registerTool(
  "process_recording",
  {
    title: "sori 녹음 받아 적기",
    description:
      "sori 앱에 녹음의 받아 적기(전사, 화자 분리)를 맡기고, 원하면 요약까지 이어서 시킨다. 앱이 꺼져 있으면 켜진다. " +
      "이미 전사가 있으면 새로 덮어쓴다. 1시간 녹음은 몇 분 걸린다. wait_seconds를 주면 그만큼 끝나길 기다리고, 0이면 시작만 확인하고 돌아온다. " +
      "끝났는지는 get_recording의 상태로 볼 수 있다. 화자 수를 알면 speakers로 넣는다(자동 추정보다 정확하다)",
    inputSchema: {
      id: z.string().min(1).describe("녹음 번호(\"12\" 또는 \"#12\")나 id"),
      speakers: z.number().int().min(1).max(20).optional().describe("화자 수. 모르면 비워 둔다(앱이 추정)"),
      languages: z
        .array(z.string().regex(/^[a-z]{2,5}$/))
        .optional()
        .describe("whisper 언어 코드. 예: [\"ko\"], 섞였으면 [\"ko\", \"en\"]. 비우면 앱 설정"),
      diarize: z.boolean().optional().describe("화자를 나눌지. 비우면 앱 설정"),
      summarize: z.boolean().optional().describe("받아 적은 뒤 요약까지 할지. 비우면 앱 설정의 자동 요약"),
      wait_seconds: z.number().int().min(0).max(900).default(0).describe("끝날 때까지 기다릴 최대 시간(초)"),
    },
    annotations: { readOnlyHint: false, destructiveHint: true, idempotentHint: false, openWorldHint: false },
  },
  async ({ id, speakers, languages, diarize, summarize, wait_seconds }) => {
    const r = await findRecording(id);
    if (!r) return notFound(id);
    if (r.status === "recording") return fail(`#${r.number} ${r.title}은 아직 녹음 중입니다. 녹음을 멈춘 뒤 다시 요청해 주세요.`);
    if (r.status === "processing") return fail(`#${r.number} ${r.title}은 이미 처리 중입니다. get_recording으로 상태를 확인해 주세요.`);
    const token = await linkToken();
    if (!token) return fail(NO_TOKEN);

    const q = new URLSearchParams({ id: r.id });
    if (speakers) q.set("speakers", String(speakers));
    for (const l of languages ?? []) q.append("lang", l);
    if (diarize !== undefined) q.set("diarize", diarize ? "1" : "0");
    if (summarize !== undefined) q.set("summarize", summarize ? "1" : "0");
    const before = { transcript: await transcriptTime(r.id), summary: await summaryTime(r.id) };
    q.set("token", token);
    await openLink(`sori://process?${q}`);

    // 앱이 받으면 곧바로 상태를 "처리 중"으로 바꾼다. 앱을 새로 켜는 경우를 생각해 넉넉히 기다린다
    const started = await waitUntil(async () => {
      const m = await getRecording(r.id);
      return m?.status === "processing" || (await transcriptTime(r.id)) > before.transcript;
    }, 45);
    if (!started) return fail(NOT_RECEIVED);

    const label = `#${r.number} ${r.title}`;
    const options = [
      speakers ? `화자 ${speakers}명` : "화자 수 자동",
      languages?.length ? `언어 ${languages.join(", ")}` : "언어는 앱 설정",
    ].join(", ");
    const wantSummary = summarize ?? (await autoSummarize());
    if (wait_seconds === 0) {
      return text(
        `${label} 받아 적기를 시작했습니다(${options}${wantSummary ? ", 끝나면 요약" : ""}). ` +
          `길이 ${r.duration != null ? formatTime(r.duration) : "모름"}. 끝났는지는 get_recording으로 확인해 주세요.`,
      );
    }

    const t0 = Date.now();
    const finished = await waitUntil(async () => {
      const m = await getRecording(r.id);
      if (m?.status === "failed") return true;
      if (m?.status !== "done") return false;
      return !wantSummary || (await summaryTime(r.id)) !== before.summary;
    }, wait_seconds);
    const m = await getRecording(r.id);
    const took = formatTime((Date.now() - t0) / 1000);
    if (m?.status === "failed") return fail(`${label} 처리에 실패했습니다: ${m.error ?? "이유 모름"}`);
    if (!finished) {
      return text(
        `${label} 아직 ${m?.status === "done" ? "요약 중" : "받아 적는 중"}입니다(${took} 기다림). 나중에 get_recording으로 확인해 주세요.`,
      );
    }
    return text(`${label} ${wantSummary ? "받아 적기와 요약을" : "받아 적기를"} 마쳤습니다(${took}). get_recording으로 읽을 수 있습니다.`);
  },
);

server.registerTool(
  "summarize_recording",
  {
    title: "sori 녹음 요약",
    description:
      "이미 받아 적은 녹음을 sori 앱에 다시 요약시킨다. 요약 방식(Claude, Codex)과 템플릿은 앱 설정을 따른다. " +
      "전사가 없으면 process_recording에 summarize: true로 요청한다. wait_seconds를 주면 그만큼 끝나길 기다린다",
    inputSchema: {
      id: z.string().min(1).describe("녹음 번호(\"12\" 또는 \"#12\")나 id"),
      wait_seconds: z.number().int().min(0).max(600).default(0).describe("끝날 때까지 기다릴 최대 시간(초)"),
    },
    annotations: { readOnlyHint: false, destructiveHint: false, idempotentHint: false, openWorldHint: false },
  },
  async ({ id, wait_seconds }) => {
    const r = await findRecording(id);
    if (!r) return notFound(id);
    const label = `#${r.number} ${r.title}`;
    if (r.status === "processing") return fail(`${label}은 받아 적는 중입니다. 끝난 뒤 요청해 주세요.`);
    if (!(await getTranscript(r.id))) {
      return fail(`${label}은 아직 전사가 없습니다. process_recording에 summarize: true로 요청해 주세요.`);
    }
    const token = await linkToken();
    if (!token) return fail(NO_TOKEN);
    const before = await summaryTime(r.id);
    await openLink(`sori://summarize?${new URLSearchParams({ id: r.id, token })}`);
    if (wait_seconds === 0) return text(`${label} 요약을 요청했습니다. 끝나면 get_recording에서 볼 수 있습니다.`);
    const done = await waitUntil(async () => (await summaryTime(r.id)) !== before, wait_seconds);
    return done
      ? text(`${label} 요약을 마쳤습니다. get_recording으로 읽을 수 있습니다.`)
      : text(`${label} 아직 요약 중입니다. 앱에 오류 알림이 떴다면 요약 설정(Claude, Codex 로그인)을 확인해 주세요.`);
  },
);

await server.connect(new StdioServerTransport());

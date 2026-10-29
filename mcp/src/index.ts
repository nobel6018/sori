#!/usr/bin/env node
import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";
import { z } from "zod";
import { formatTime, getSummary, getTranscript, listRecordings, normalize, Recording, speakerName, Transcript } from "./store.js";

// sori 녹음을 MCP 클라이언트(Claude Code, Claude 데스크톱 등)에서 찾고 읽는다. 읽기 전용이다
// @see ../README.md

const server = new McpServer({ name: "sori", version: "0.4.4" });

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
    const key = id.trim().replace(/^#/, "");
    const all = await listRecordings();
    const r = /^\d+$/.test(key) ? all.find((x) => x.number === Number(key)) : all.find((x) => x.id === key);
    if (!r) return { ...text(`${id} 녹음이 없습니다. list_recordings로 번호를 확인해 주세요.`), isError: true };
    const lines = [`# ${r.title}`, "", `- 번호: #${r.number ?? "?"}`, `- 날짜: ${dateOf(r)}`];
    if (r.duration != null) lines.push(`- 길이: ${formatTime(r.duration)}`);
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

await server.connect(new StdioServerTransport());

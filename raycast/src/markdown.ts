import { Detail, formatTime, Recording, speakerName, Utterance } from "./store";

// sori 앱의 src/markdown.ts와 같은 형식이다. 바꾸면 함께 바꾼다

export interface Block {
  speaker: number;
  start: number;
  end: number;
  texts: string[];
}

export function toBlocks(utterances: Utterance[]): Block[] {
  const blocks: Block[] = [];
  for (const u of utterances) {
    const last = blocks[blocks.length - 1];
    if (last && last.speaker === u.speaker) {
      last.texts.push(u.text.trim());
      last.end = u.end;
    } else {
      blocks.push({ speaker: u.speaker, start: u.start, end: u.end, texts: [u.text.trim()] });
    }
  }
  return blocks;
}

export function transcriptMarkdown(recording: Recording, utterances: Utterance[]): string {
  return toBlocks(utterances)
    .map((b) => `**[${formatTime(b.start)}] ${speakerName(recording, b.speaker)}** ${b.texts.join(" ")}`)
    .join("\n\n");
}

export function recordingMarkdown({ recording, transcript, summary }: Detail): string {
  const lines = [`# ${recording.title}`, ""];
  const date = new Date(recording.createdAt).toLocaleString("ko-KR", { dateStyle: "medium", timeStyle: "short" });
  lines.push(`- 날짜: ${date}`);
  if (recording.duration != null) lines.push(`- 길이: ${formatTime(recording.duration)}`);
  if (transcript) {
    const speakers = [...new Set(transcript.utterances.map((u) => u.speaker))].sort((a, b) => a - b);
    const names = speakers.map((s) => speakerName(recording, s) + (transcript.origins?.[String(s)] === "remote" ? " (원격)" : ""));
    if (names.length > 0) lines.push(`- 참석: ${names.join(", ")}`);
  }
  if (summary) lines.push("", "## 요약", "", summary.markdown.trim().replace(/^(#{1,5}) /gm, "#$1 "));
  if (recording.notes?.length) {
    lines.push("", "## 메모", "", ...recording.notes.map((n) => `- [${formatTime(n.time)}] ${n.text || "(북마크)"}`));
  }
  if (transcript && transcript.utterances.length > 0) {
    lines.push("", "## 전사", "", transcriptMarkdown(recording, transcript.utterances));
  }
  return lines.join("\n") + "\n";
}

/** 대소문자와 띄어쓰기를 무시한다. src-tauri/src/search.rs normalize와 같다 */
export function normalize(s: string): string {
  return s.replace(/\s+/g, "").toLowerCase();
}

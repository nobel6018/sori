import { formatTime, RecordingDetail, Recording, speakerName, Utterance } from "./api";

export interface Block {
  speaker: number;
  start: number;
  end: number;
  utterances: Utterance[];
}

/** 같은 화자가 이어서 말한 발화를 한 덩어리로 묶는다 */
export function toBlocks(utterances: Utterance[]): Block[] {
  const blocks: Block[] = [];
  for (const u of utterances) {
    const last = blocks[blocks.length - 1];
    if (last && last.speaker === u.speaker) {
      last.utterances.push(u);
      last.end = u.end;
    } else {
      blocks.push({ speaker: u.speaker, start: u.start, end: u.end, utterances: [u] });
    }
  }
  return blocks;
}

export function transcriptMarkdown(recording: Recording, utterances: Utterance[]): string {
  return toBlocks(utterances)
    .map((b) => `**[${formatTime(b.start)}] ${speakerName(recording, b.speaker)}** ${b.utterances.map((u) => u.text.trim()).join(" ")}`)
    .join("\n\n");
}

/**
 * 제목, 정보, 요약, 전사를 한 문서로 만든다. 슬랙이나 노션에 바로 붙여 넣는 용도다.
 * 요약의 제목 단계는 한 단계씩 내려 문서 제목(#) 아래에 들어가게 한다.
 * raycast/src/markdown.ts에 같은 형식이 있으니 바꾸면 함께 바꾼다
 */
export function recordingMarkdown(detail: RecordingDetail): string {
  const { recording, transcript, summary } = detail;
  const lines = [`# ${recording.title}`, ""];
  const date = new Date(recording.createdAt).toLocaleString("ko-KR", { dateStyle: "medium", timeStyle: "short" });
  lines.push(`- 날짜: ${date}`);
  if (recording.duration != null) lines.push(`- 길이: ${formatTime(recording.duration)}`);
  if (transcript) {
    const speakers = [...new Set(transcript.utterances.map((u) => u.speaker))].sort((a, b) => a - b);
    const names = speakers.map((s) => speakerName(recording, s) + (transcript.origins?.[String(s)] === "remote" ? " (원격)" : ""));
    if (names.length > 0) lines.push(`- 참석: ${names.join(", ")}`);
  }
  if (summary) {
    lines.push("", "## 요약", "", summary.markdown.trim().replace(/^(#{1,5}) /gm, "#$1 "));
  }
  if (transcript && transcript.utterances.length > 0) {
    lines.push("", "## 전사", "", transcriptMarkdown(recording, transcript.utterances));
  }
  return lines.join("\n") + "\n";
}

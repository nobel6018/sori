import { formatTime, RecordingDetail, speakerName, Utterance } from "./api";
import { recordingMarkdown, toBlocks } from "./markdown";

// 녹음을 파일로 내보낸다. 자막(SRT, VTT)은 발화 하나를 자막 한 칸으로 쓴다
// @see docs/export.md

export type ExportFormat = "md" | "txt" | "srt" | "vtt" | "docx";

export const EXPORT_LABEL: Record<ExportFormat, string> = {
  md: "마크다운 (.md)",
  txt: "텍스트 (.txt)",
  srt: "자막 SRT (.srt)",
  vtt: "자막 WebVTT (.vtt)",
  docx: "Word 문서 (.docx)",
};

/** 자막 한 칸이 이보다 길면 문장 경계에서 나눈다. 화면 아래 두 줄 정도가 읽기 좋다 */
const MAX_CUE_CHARS = 84;

function stamp(seconds: number, sep: "," | "."): string {
  const ms = Math.max(0, Math.round(seconds * 1000));
  const h = Math.floor(ms / 3_600_000);
  const m = Math.floor((ms % 3_600_000) / 60_000);
  const s = Math.floor((ms % 60_000) / 1000);
  const pad = (n: number, w = 2) => String(n).padStart(w, "0");
  return `${pad(h)}:${pad(m)}:${pad(s)}${sep}${pad(ms % 1000, 3)}`;
}

/** 긴 발화를 문장 단위로 나누고, 시간은 글자 수 비율로 나눠 준다 */
function cues(utterances: Utterance[]): { start: number; end: number; speaker: number; text: string }[] {
  const out: { start: number; end: number; speaker: number; text: string }[] = [];
  for (const u of utterances) {
    const text = u.text.trim();
    if (!text) continue;
    const sentences = text.match(/[^.!?。！？]+[.!?。！？]*\s*/g) ?? [text];
    const parts: string[] = [];
    for (const s of sentences) {
      const last = parts[parts.length - 1];
      if (last !== undefined && (last + s).length <= MAX_CUE_CHARS) parts[parts.length - 1] = last + s;
      else parts.push(s);
    }
    const total = parts.reduce((n, p) => n + p.length, 0) || 1;
    let t = u.start;
    for (const p of parts) {
      const end = t + ((u.end - u.start) * p.length) / total;
      out.push({ start: t, end, speaker: u.speaker, text: p.trim() });
      t = end;
    }
  }
  return out;
}

function subtitles(detail: RecordingDetail, kind: "srt" | "vtt"): string {
  const { recording, transcript } = detail;
  const list = cues(transcript?.utterances ?? []);
  const body = list.map((c, i) => {
    const name = speakerName(recording, c.speaker);
    const time = `${stamp(c.start, kind === "srt" ? "," : ".")} --> ${stamp(c.end, kind === "srt" ? "," : ".")}`;
    // VTT는 화자 표시 태그(<v 이름>)가 있어 플레이어가 화자를 따로 보여 줄 수 있다
    const text = kind === "vtt" ? `<v ${name}>${c.text}` : `${name}: ${c.text}`;
    return kind === "srt" ? `${i + 1}\n${time}\n${text}` : `${time}\n${text}`;
  });
  return kind === "vtt" ? `WEBVTT\n\n${body.join("\n\n")}\n` : `${body.join("\n\n")}\n`;
}

function plainText(detail: RecordingDetail): string {
  const { recording, transcript } = detail;
  return (
    toBlocks(transcript?.utterances ?? [])
      .map((b) => `[${formatTime(b.start)}] ${speakerName(recording, b.speaker)}: ${b.utterances.map((u) => u.text.trim()).join(" ")}`)
      .join("\n\n") + "\n"
  );
}

/** 요약 마크다운의 제목, 목록, 굵은 글씨만 Word 서식으로 옮긴다 */
async function docx(detail: RecordingDetail): Promise<Uint8Array> {
  const { Document, HeadingLevel, Packer, Paragraph, TextRun } = await import("docx");
  const { recording, transcript, summary } = detail;
  const runs = (line: string, base: { bold?: boolean } = {}) =>
    line.split(/(\*\*[^*]+\*\*)/g).filter(Boolean).map((part) =>
      part.startsWith("**") && part.endsWith("**")
        ? new TextRun({ text: part.slice(2, -2), bold: true })
        : new TextRun({ text: part, ...base }),
    );
  const children: InstanceType<typeof Paragraph>[] = [new Paragraph({ text: recording.title, heading: HeadingLevel.TITLE })];
  const info = [new Date(recording.createdAt).toLocaleString("ko-KR")];
  if (recording.duration != null) info.push(`길이 ${formatTime(recording.duration)}`);
  children.push(new Paragraph({ children: [new TextRun({ text: info.join(", "), color: "777777" })] }));

  if (summary) {
    children.push(new Paragraph({ text: "요약", heading: HeadingLevel.HEADING_1 }));
    for (const raw of summary.markdown.split("\n")) {
      const line = raw.trimEnd();
      if (!line.trim()) continue;
      const heading = line.match(/^(#{1,6})\s+(.*)$/);
      const bullet = line.match(/^(\s*)[-*+]\s+(?:\[[ x]\]\s+)?(.*)$/);
      const numbered = line.match(/^\s*\d+\.\s+(.*)$/);
      if (heading) {
        const level = heading[1].length <= 2 ? HeadingLevel.HEADING_2 : HeadingLevel.HEADING_3;
        children.push(new Paragraph({ text: heading[2].replace(/\*\*/g, ""), heading: level }));
      } else if (bullet) {
        children.push(new Paragraph({ children: runs(bullet[2]), bullet: { level: Math.min(Math.floor(bullet[1].length / 2), 3) } }));
      } else if (numbered) {
        children.push(new Paragraph({ children: runs(line.trim()) }));
      } else {
        children.push(new Paragraph({ children: runs(line) }));
      }
    }
  }

  if (recording.notes?.length) {
    children.push(new Paragraph({ text: "메모", heading: HeadingLevel.HEADING_1 }));
    for (const n of recording.notes) {
      children.push(new Paragraph({ text: `[${formatTime(n.time)}] ${n.text || "(북마크)"}`, bullet: { level: 0 } }));
    }
  }

  if (transcript?.utterances.length) {
    children.push(new Paragraph({ text: "전사", heading: HeadingLevel.HEADING_1 }));
    for (const b of toBlocks(transcript.utterances)) {
      children.push(
        new Paragraph({
          spacing: { before: 160 },
          children: [
            new TextRun({ text: speakerName(recording, b.speaker), bold: true }),
            new TextRun({ text: `  ${formatTime(b.start)}`, color: "777777" }),
          ],
        }),
      );
      children.push(new Paragraph({ text: b.utterances.map((u) => u.text.trim()).join(" ") }));
    }
  }

  const doc = new Document({
    creator: "sori",
    title: recording.title,
    styles: { default: { document: { run: { font: "Malgun Gothic" } } } },
    sections: [{ children }],
  });
  return new Uint8Array(await (await Packer.toBlob(doc)).arrayBuffer());
}

export async function exportBytes(detail: RecordingDetail, format: ExportFormat): Promise<Uint8Array> {
  if (format === "docx") return docx(detail);
  const text =
    format === "md"
      ? recordingMarkdown(detail)
      : format === "txt"
        ? plainText(detail)
        : subtitles(detail, format);
  return new TextEncoder().encode(text);
}

/** 파일 이름에 쓸 수 없는 글자를 뺀다 */
export function exportFileName(title: string, format: ExportFormat): string {
  const safe = title.replace(/[\\/:*?"<>|]/g, " ").trim() || "sori";
  return `${safe}.${format}`;
}


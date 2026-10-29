import { Action, ActionPanel, Clipboard, Color, Icon, List, showHUD, showToast, Toast } from "@raycast/api";
import { useEffect, useMemo, useState } from "react";
import { normalize, recordingMarkdown, toBlocks, transcriptMarkdown } from "./markdown";
import { Detail, formatTime, getDetail, getSummary, getTranscript, listRecordings, openSori, Recording, speakerName } from "./store";

interface Hit {
  kind: "summary" | "utterance";
  start: number | null;
  speaker: number | null;
  text: string;
}

interface Group {
  recording: Recording;
  hits: Hit[];
}

const STATUS: Record<Recording["status"], { text: string; color: Color }> = {
  recording: { text: "녹음 중", color: Color.Red },
  new: { text: "대기", color: Color.SecondaryText },
  processing: { text: "처리 중", color: Color.Yellow },
  done: { text: "완료", color: Color.Green },
  failed: { text: "실패", color: Color.Orange },
};

/** 녹음 하나에서 보여 줄 최대 결과 수 */
const HITS_PER_RECORDING = 20;

async function search(recordings: Recording[], query: string): Promise<Group[]> {
  const needle = normalize(query);
  if (!needle) return recordings.map((recording) => ({ recording, hits: [] }));
  // "#12"나 "12"로 찾으면 그 번호의 녹음을 먼저 보여 준다
  const asNumber = /^#?\d+$/.test(needle) ? Number(needle.replace("#", "")) : null;
  const groups = await Promise.all(
    recordings.map(async (recording) => {
      const [transcript, summary] = await Promise.all([getTranscript(recording.id), getSummary(recording.id)]);
      const hits: Hit[] = [];
      for (const line of summary?.markdown.split("\n") ?? []) {
        const text = line.replace(/^[\s#>*+-]*(\d+\.\s)?/, "").replace(/\*\*|`/g, "").trim();
        if (text && normalize(text).includes(needle)) hits.push({ kind: "summary", start: null, speaker: null, text });
      }
      for (const u of transcript?.utterances ?? []) {
        if (normalize(u.text).includes(needle)) hits.push({ kind: "utterance", start: u.start, speaker: u.speaker, text: u.text.trim() });
      }
      const titleMatch = recording.number === asNumber || normalize(recording.title).includes(needle);
      return titleMatch || hits.length > 0 ? { recording, hits: hits.slice(0, HITS_PER_RECORDING) } : null;
    }),
  );
  return groups.filter((g): g is Group => g !== null);
}

/** 미리 보기. 찾은 문장이 있으면 그 근처 전사를, 없으면 요약(없으면 전사 앞부분)을 보여 준다 */
function previewMarkdown(detail: Detail | undefined, focus: number | null): string {
  if (!detail) return "";
  const { recording, transcript, summary } = detail;
  const head = `## ${recording.title}\n\n`;
  if (!transcript) return `${head}아직 받아 적지 않은 녹음입니다.`;
  const blocks = toBlocks(transcript.utterances);
  const at = focus == null ? 0 : Math.max(0, blocks.findIndex((b) => b.end > focus));
  const excerpt = blocks
    .slice(Math.max(0, at - 2), at + 10)
    .map((b, i, all) => {
      const text = b.texts.join(" ");
      const isFocus = focus != null && blocks.indexOf(all[i]) === at;
      return `**${speakerName(recording, b.speaker)}** \`${formatTime(b.start)}\`\n\n${isFocus ? `> ${text}` : text}`;
    })
    .join("\n\n");
  if (focus == null && summary) return head + summary.markdown;
  return head + excerpt;
}

export function Recordings({ searchable }: { searchable: boolean }) {
  const [recordings, setRecordings] = useState<Recording[] | null>(null);
  const [query, setQuery] = useState("");
  const [groups, setGroups] = useState<Group[] | null>(null);
  const [details, setDetails] = useState<Record<string, Detail>>({});
  const [selected, setSelected] = useState<string | null>(null);

  useEffect(() => {
    listRecordings().then(setRecordings);
  }, []);

  useEffect(() => {
    if (!recordings) return;
    let stale = false;
    const timer = setTimeout(() => search(recordings, query).then((g) => !stale && setGroups(g)), query ? 150 : 0);
    return () => {
      stale = true;
      clearTimeout(timer);
    };
  }, [recordings, query]);

  const byId = useMemo(() => new Map((recordings ?? []).map((r) => [r.id, r])), [recordings]);
  const selectedRecording = selected ? byId.get(selected.split("#")[0]) : undefined;

  useEffect(() => {
    if (!selectedRecording || details[selectedRecording.id]) return;
    getDetail(selectedRecording).then((d) => setDetails((prev) => ({ ...prev, [d.recording.id]: d })));
  }, [selectedRecording, details]);

  const detailOf = async (r: Recording) => details[r.id] ?? (await getDetail(r));

  const copy = async (r: Recording, what: "markdown" | "summary" | "transcript") => {
    const d = await detailOf(r);
    const text =
      what === "markdown" ? recordingMarkdown(d) : what === "summary" ? (d.summary?.markdown ?? "") : d.transcript ? transcriptMarkdown(r, d.transcript.utterances) : "";
    if (!text) {
      await showToast({ style: Toast.Style.Failure, title: what === "summary" ? "요약이 없습니다" : "전사가 없습니다" });
      return;
    }
    await Clipboard.copy(text);
    await showHUD("복사했습니다");
  };

  const open = async (r: Recording, time: number | null) => {
    try {
      await openSori("open", time == null ? [["id", r.id]] : [["id", r.id], ["t", time.toFixed(2)]]);
    } catch (e) {
      await showToast({ style: Toast.Style.Failure, title: String(e) });
    }
  };

  const actions = (r: Recording, hit?: Hit) => (
    <ActionPanel>
      <Action title="Open in Sori" icon={Icon.AppWindow} onAction={() => open(r, hit?.start ?? null)} />
      <Action title="Copy as Markdown" icon={Icon.Clipboard} shortcut={{ modifiers: ["cmd", "shift"], key: "c" }} onAction={() => copy(r, "markdown")} />
      <Action title="Copy Summary" icon={Icon.Text} shortcut={{ modifiers: ["cmd", "shift"], key: "s" }} onAction={() => copy(r, "summary")} />
      <Action title="Copy Transcript" icon={Icon.Document} shortcut={{ modifiers: ["cmd", "shift"], key: "t" }} onAction={() => copy(r, "transcript")} />
      {hit && <Action.CopyToClipboard title="Copy This Sentence" content={hit.text} shortcut={{ modifiers: ["cmd"], key: "." }} />}
    </ActionPanel>
  );

  const focusOf = (key: string | null) => {
    if (!key || !key.includes("#")) return null;
    const [id, index] = key.split("#");
    const hit = groups?.find((g) => g.recording.id === id)?.hits[Number(index)];
    return hit?.start ?? null;
  };
  const markdown = previewMarkdown(selectedRecording ? details[selectedRecording.id] : undefined, focusOf(selected));

  return (
    <List
      isLoading={groups === null}
      isShowingDetail
      filtering={!searchable}
      onSearchTextChange={searchable ? setQuery : undefined}
      searchBarPlaceholder={searchable ? "제목, 요약, 전사에서 찾기" : "최근 녹음"}
      onSelectionChange={setSelected}
      throttle
    >
      {groups?.length === 0 && <List.EmptyView title={query ? "찾은 내용이 없습니다" : "녹음이 없습니다"} />}
      {query
        ? groups?.map((g) => (
            <List.Section key={g.recording.id} title={g.recording.title} subtitle={new Date(g.recording.createdAt).toLocaleDateString("ko-KR")}>
              <List.Item id={g.recording.id} title={g.recording.title} icon={Icon.Microphone} detail={<List.Item.Detail markdown={markdown} />} actions={actions(g.recording)} />
              {g.hits.map((h, i) => (
                <List.Item
                  key={i}
                  id={`${g.recording.id}#${i}`}
                  title={h.text}
                  icon={h.kind === "summary" ? Icon.Text : Icon.SpeechBubble}
                  accessories={[{ text: h.kind === "summary" ? "요약" : `${formatTime(h.start ?? 0)} ${speakerName(g.recording, h.speaker ?? 0)}` }]}
                  detail={<List.Item.Detail markdown={markdown} />}
                  actions={actions(g.recording, h)}
                />
              ))}
            </List.Section>
          ))
        : groups?.map(({ recording: r }) => (
            <List.Item
              key={r.id}
              id={r.id}
              title={r.title}
              icon={Icon.Microphone}
              accessories={[
                ...(r.number ? [{ text: `#${r.number}` }] : []),
                { tag: { value: STATUS[r.status].text, color: STATUS[r.status].color } },
                ...(r.duration != null ? [{ text: formatTime(r.duration) }] : []),
              ]}
              keywords={[new Date(r.createdAt).toLocaleDateString("ko-KR"), ...(r.number ? [`#${r.number}`, String(r.number)] : [])]}
              detail={<List.Item.Detail markdown={markdown} />}
              actions={actions(r)}
            />
          ))}
    </List>
  );
}

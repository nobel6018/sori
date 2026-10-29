import { memo, useEffect, useMemo, useRef } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { formatTime, Note, Recording, speakerName } from "../api";
import { Block } from "../markdown";

interface Props {
  blocks: Block[];
  recording: Recording;
  currentTime: number;
  /** 재생 중이고 자동 스크롤이 켜져 있을 때만 true */
  follow: boolean;
  /** 검색 결과로 열었을 때 한 번 옮겨 갈 시각 */
  scrollTarget: { time: number; nonce: number } | null;
  /** 블록 안에 메모 표시를 단다 */
  notes: Note[];
  onSeek: (time: number) => void;
}

/** 지금 재생 위치가 들어 있는 블록. 블록은 시간순이라 이진 탐색한다 */
function findActive(blocks: Block[], time: number): number {
  let lo = 0;
  let hi = blocks.length - 1;
  while (lo <= hi) {
    const mid = (lo + hi) >> 1;
    if (time < blocks[mid].start) hi = mid - 1;
    else if (time >= blocks[mid].end) lo = mid + 1;
    else return mid;
  }
  return -1;
}

/**
 * 화면에 보이는 블록만 그린다. 1시간 회의면 블록이 수백 개라,
 * 재생 중 0.25초마다 전부 다시 그리면 스크롤이 끊긴다
 */
export function TranscriptList({ blocks, recording, currentTime, follow, scrollTarget, notes, onSeek }: Props) {
  // 블록마다 그 사이에 남긴 메모. 다음 블록이 시작하기 전까지를 그 블록 몫으로 친다
  const notesByBlock = useMemo(() => {
    const map = new Map<number, Note[]>();
    for (const n of notes) {
      let i = 0;
      while (i + 1 < blocks.length && blocks[i + 1].start <= n.time) i++;
      map.set(i, [...(map.get(i) ?? []), n]);
    }
    return map;
  }, [blocks, notes]);

  const scrollRef = useRef<HTMLDivElement>(null);
  const virtualizer = useVirtualizer({
    count: blocks.length,
    getScrollElement: () => scrollRef.current,
    // 실제 높이는 그린 뒤 measureElement로 잰다. 처음 스크롤바 길이만 이 값으로 어림한다
    estimateSize: () => 80,
    overscan: 6,
  });

  const active = findActive(blocks, currentTime);

  // 재생 중 말하는 블록이 화면 밖으로 나가면 따라간다. 화면 안이면 움직이지 않는다
  useEffect(() => {
    if (follow && active >= 0) virtualizer.scrollToIndex(active, { align: "auto" });
  }, [active, follow, virtualizer]);

  useEffect(() => {
    if (!scrollTarget) return;
    const index = findActive(blocks, scrollTarget.time);
    // 발화 사이 빈 구간이면 그 뒤 첫 블록으로 간다
    const target = index >= 0 ? index : blocks.findIndex((b) => b.start >= scrollTarget.time);
    if (target >= 0) virtualizer.scrollToIndex(target, { align: "center" });
  }, [scrollTarget, blocks, virtualizer]);

  if (blocks.length === 0) return <p className="muted">받아 적은 내용이 없습니다.</p>;

  return (
    // tabIndex: 키보드로도 PageDown, End 등으로 전사를 넘겨 볼 수 있게 포커스를 받는다
    <div ref={scrollRef} className="transcript-scroll" tabIndex={0} aria-label="전사">
      <div className="transcript" style={{ height: virtualizer.getTotalSize(), position: "relative" }}>
        {virtualizer.getVirtualItems().map((item) => (
          <div
            key={item.key}
            data-index={item.index}
            ref={virtualizer.measureElement}
            className="transcript-row"
            style={{ transform: `translateY(${item.start}px)` }}
          >
            <BlockView
              block={blocks[item.index]}
              name={speakerName(recording, blocks[item.index].speaker)}
              // 지금 블록만 재생 위치를 받아, 나머지 블록은 다시 그리지 않는다
              currentTime={item.index === active ? currentTime : -1}
              notes={notesByBlock.get(item.index)}
              onSeek={onSeek}
            />
          </div>
        ))}
      </div>
    </div>
  );
}

const BlockView = memo(function BlockView({
  block,
  name,
  currentTime,
  notes,
  onSeek,
}: {
  block: Block;
  name: string;
  currentTime: number;
  notes?: Note[];
  onSeek: (time: number) => void;
}) {
  const active = currentTime >= 0;
  return (
    <article className={`block ${active ? "active" : ""}`}>
      <div className="block-head">
        <span className={`speaker-name spk-${block.speaker % 8}`}>{name}</span>
        <button className="time" onClick={() => onSeek(block.start)}>
          {formatTime(block.start)}
        </button>
        {notes?.map((n, i) => (
          <button key={i} className="note-marker" onClick={() => onSeek(n.time)} title={`${formatTime(n.time)} 메모`}>
            {n.text || "북마크"}
          </button>
        ))}
      </div>
      <p className="block-text">
        {block.utterances.map((u, i) => (
          <span
            key={i}
            className={active && currentTime >= u.start && currentTime < u.end ? "now" : ""}
            onDoubleClick={() => onSeek(u.start)}
          >
            {u.text}{" "}
          </span>
        ))}
      </p>
    </article>
  );
});

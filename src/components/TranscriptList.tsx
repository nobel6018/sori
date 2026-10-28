import { memo, useEffect, useRef } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { formatTime, Recording, speakerName, Utterance } from "../api";

export interface Block {
  speaker: number;
  start: number;
  end: number;
  utterances: Utterance[];
}

interface Props {
  blocks: Block[];
  recording: Recording;
  currentTime: number;
  playing: boolean;
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
export function TranscriptList({ blocks, recording, currentTime, playing, onSeek }: Props) {
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
    if (playing && active >= 0) virtualizer.scrollToIndex(active, { align: "auto" });
  }, [active, playing, virtualizer]);

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
  onSeek,
}: {
  block: Block;
  name: string;
  currentTime: number;
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

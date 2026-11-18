import { memo, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { convertFileSrc } from "@tauri-apps/api/core";
import Markdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { formatBytes, formatTime, startedAt } from "../../src/api";
import { Block, findActive, toBlocks } from "../../src/markdown";
import { TagEditor } from "../../src/components/TagEditor";
import { mapUrl } from "../../src/tags";
import { api, Recording, RecordingDetail } from "./api";

type Tab = "transcript" | "summary" | "notes";

function speakerName(meta: Recording, speaker: number): string {
  return meta.speakerNames?.[String(speaker)] ?? `화자 ${speaker + 1}`;
}

export function Detail({ id, allTags, onBack }: { id: string; allTags: [string, number][]; onBack: () => void }) {
  const [detail, setDetail] = useState<RecordingDetail | null>(null);
  const [tab, setTab] = useState<Tab>("transcript");
  const [error, setError] = useState<string | null>(null);
  const audio = useRef<HTMLAudioElement | null>(null);
  // 전사 화면이 재생 위치를 따라가려면 오디오 요소가 붙은 때를 알아야 해서 상태로도 둔다
  const [audioEl, setAudioEl] = useState<HTMLAudioElement | null>(null);
  const attachAudio = useCallback((el: HTMLAudioElement | null) => {
    audio.current = el;
    setAudioEl(el);
  }, []);

  const load = useCallback(() => {
    api.getRecording(id).then(setDetail).catch((e) => setError(String(e)));
  }, [id]);

  useEffect(() => {
    load();
    const sub = api.onRecordingChanged((changed) => changed === id && load());
    const synced = api.onRecordingsSynced(load);
    return () => {
      sub.then((f) => f());
      synced.then((f) => f());
    };
  }, [id, load]);

  const seek = (t: number) => {
    if (!audio.current) return;
    audio.current.currentTime = t;
    audio.current.play().catch(() => {});
  };

  if (!detail) {
    return (
      <div className="screen">
        <header className="bar">
          <button className="text-button" onClick={onBack}>
            ‹ 목록
          </button>
        </header>
        {error && <p className="error pad">{error}</p>}
      </div>
    );
  }

  const meta = detail.meta;
  const remove = async () => {
    if (!confirm(`"${meta.title}" 녹음을 지울까요? 동기화하면 다른 기기에서도 지워집니다.`)) return;
    try {
      await api.deleteRecording(id);
      onBack();
    } catch (e) {
      setError(String(e));
    }
  };

  return (
    <div className="screen">
      <header className="bar">
        <button className="text-button" onClick={onBack}>
          ‹ 목록
        </button>
        <button className="text-button danger" onClick={remove}>
          삭제
        </button>
      </header>

      <main className="content detail">
        <Title meta={meta} onError={setError} />
        <p className="meta-line muted small">
          <span>#{meta.number}</span>
          <span>{startedAt(meta).toLocaleString("ko-KR", { dateStyle: "medium", timeStyle: "short" })}</span>
          {meta.duration != null && <span>{formatTime(meta.duration)}</span>}
        </p>
        <TagEditor
          tags={meta.tags ?? []}
          allTags={allTags}
          aiTagged={!!meta.autoTaggedAt && !meta.tagsEditedAt}
          onChange={(tags) =>
            api
              .setTags(id, tags)
              .then(load)
              .catch((e) => setError(String(e)))
          }
        />
        {meta.event && <p className="muted small event-line">일정: {meta.event.title}</p>}
        {meta.location && (
          <button className="place-link" onClick={() => api.openUrl(mapUrl(meta.location!)).catch((e) => setError(String(e)))}>
            녹음한 곳: {meta.location.place || `${meta.location.lat.toFixed(4)}, ${meta.location.lon.toFixed(4)}`} ›
          </button>
        )}
        {error && <p className="error">{error}</p>}

        {detail.audioAvailable ? (
          <audio ref={attachAudio} className="player" controls preload="metadata" src={convertFileSrc(detail.audioPath)} />
        ) : meta.status === "finishing" ? (
          <p className="muted small">
            녹음 파일을 저장하는 중입니다.
            {meta.finishError && ` 저장하지 못했지만 녹음은 이 iPhone에 남아 있고, 앱을 다시 열면 다시 저장합니다. (${meta.finishError})`}
          </p>
        ) : (
          <AudioDownload id={id} onDone={load} />
        )}

        <nav className="tabs" role="tablist">
          {(
            [
              ["transcript", "전사"],
              ["summary", "요약"],
              ["notes", `메모${meta.notes?.length ? ` ${meta.notes.length}` : ""}`],
            ] as [Tab, string][]
          ).map(([key, label]) => (
            <button key={key} role="tab" aria-selected={tab === key} className={tab === key ? "on" : ""} onClick={() => setTab(key)}>
              {label}
            </button>
          ))}
        </nav>

        {tab === "transcript" && <TranscriptView detail={detail} audio={audioEl} onSeek={seek} onReload={load} />}
        {tab === "summary" && <SummaryView detail={detail} />}
        {tab === "notes" && (
          <NotesView detail={detail} onSeek={seek} currentTime={() => audio.current?.currentTime ?? 0} onError={setError} />
        )}
      </main>
    </div>
  );
}

function Title({ meta, onError }: { meta: Recording; onError: (e: string) => void }) {
  const [editing, setEditing] = useState(false);
  const [value, setValue] = useState(meta.title);
  if (!editing) {
    return (
      <h2 className="detail-title" onClick={() => setEditing(true)}>
        {meta.title}
      </h2>
    );
  }
  const save = async () => {
    setEditing(false);
    if (value.trim() && value.trim() !== meta.title) {
      await api.renameRecording(meta.id, value).catch((e) => onError(String(e)));
    }
  };
  return (
    <form
      onSubmit={(e) => {
        e.preventDefault();
        save();
      }}
    >
      <input className="title-input" autoFocus value={value} onChange={(e) => setValue(e.target.value)} onBlur={save} enterKeyHint="done" />
    </form>
  );
}

function AudioDownload({ id, onDone }: { id: string; onDone: () => void }) {
  const [progress, setProgress] = useState<{ done: number; total: number } | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    const sub = api.onAudioDownload((p) => p.id === id && setProgress({ done: p.done, total: p.total }));
    return () => {
      sub.then((f) => f());
    };
  }, [id]);

  const start = async () => {
    setError(null);
    setProgress({ done: 0, total: 0 });
    try {
      await api.downloadAudio(id);
      onDone();
    } catch (e) {
      setError(String(e));
      setProgress(null);
    }
  };

  return (
    <div className="audio-download">
      <span className="muted">오디오가 iCloud에만 있습니다</span>
      {progress ? (
        <span>
          받는 중 {formatBytes(progress.done)}
          {progress.total > 0 && ` / ${formatBytes(progress.total)}`}
        </span>
      ) : (
        <button onClick={start}>오디오 받기</button>
      )}
      {error && <span className="error">{error}</span>}
    </div>
  );
}

const FOLLOW_KEY = "sori.follow";

/**
 * 재생 중 말하는 블록을 칠하고, 화면 밖으로 나가면 따라 내려간다(데스크톱 자동 스크롤과 같은 설정 이름).
 * 손으로 스크롤하면 그동안은 따라가지 않고 "지금 위치로" 버튼을 띄운다. 터치 화면에서는 따라가기가
 * 몇 초마다 손가락과 싸우기 때문이다. @see docs/mobile.md
 */
function usePlaybackFollow(audio: HTMLAudioElement | null, blocks: Block[]) {
  const [active, setActive] = useState(-1);
  const [playing, setPlaying] = useState(false);
  const [follow, setFollow] = useState(() => {
    try {
      return localStorage.getItem(FOLLOW_KEY) !== "off";
    } catch {
      return true;
    }
  });
  const [detached, setDetached] = useState(false);
  const refs = useRef<(HTMLElement | null)[]>([]);

  useEffect(() => {
    if (!audio) return;
    // timeupdate는 초에 4번쯤 오지만 블록이 바뀔 때만 다시 그린다
    const update = () => setActive(findActive(blocks, audio.currentTime));
    const play = () => setPlaying(true);
    const stop = () => setPlaying(false);
    update();
    setPlaying(!audio.paused);
    audio.addEventListener("timeupdate", update);
    audio.addEventListener("seeked", update);
    audio.addEventListener("play", play);
    audio.addEventListener("pause", stop);
    audio.addEventListener("ended", stop);
    return () => {
      audio.removeEventListener("timeupdate", update);
      audio.removeEventListener("seeked", update);
      audio.removeEventListener("play", play);
      audio.removeEventListener("pause", stop);
      audio.removeEventListener("ended", stop);
    };
  }, [audio, blocks]);

  const following = follow && playing && !detached;

  // 코드로 하는 스크롤은 touchmove를 내지 않으므로 손가락으로 민 것만 잡힌다
  useEffect(() => {
    if (!follow || !playing) return;
    const leave = () => setDetached(true);
    window.addEventListener("touchmove", leave, { passive: true });
    return () => window.removeEventListener("touchmove", leave);
  }, [follow, playing]);

  const scrollToActive = useCallback(
    (always: boolean) => {
      const el = refs.current[active];
      if (!el) return;
      // 위에 붙어 있는 막대와 탭 아래부터가 보이는 곳이다
      const tabs = document.querySelector<HTMLElement>(".tabs");
      const top = tabs?.getBoundingClientRect().bottom ?? 0;
      const box = el.getBoundingClientRect();
      if (always || box.top < top || box.bottom > window.innerHeight) {
        // 블록 시작을 탭 바로 아래에 둔다. 다음 블록들이 아래에 보여 스크롤이 덜 잦다.
        // 맨 위에서 내려가면 탭이 스크롤 도중에 위로 붙으므로, 지금 위치가 아니라 붙었을 때 위치로 잰다
        const stuck = tabs ? parseFloat(getComputedStyle(tabs).top) + tabs.offsetHeight : 0;
        window.scrollBy({ top: box.top - stuck - 8, behavior: "smooth" });
      }
    },
    [active],
  );

  useEffect(() => {
    if (following && active >= 0) scrollToActive(false);
  }, [following, active, scrollToActive]);

  const toggleFollow = () => {
    const next = !follow;
    setFollow(next);
    setDetached(false);
    try {
      localStorage.setItem(FOLLOW_KEY, next ? "on" : "off");
    } catch {
      // 저장하지 못해도 이번 화면에서는 켜고 끈 대로 동작한다
    }
  };

  const resume = () => {
    setDetached(false);
    scrollToActive(true);
  };

  return { active, refs, follow, toggleFollow, showResume: follow && playing && detached && active >= 0, resume };
}

function TranscriptView({
  detail,
  audio,
  onSeek,
  onReload,
}: {
  detail: RecordingDetail;
  audio: HTMLAudioElement | null;
  onSeek: (t: number) => void;
  onReload: () => void;
}) {
  const blocks = useMemo(() => toBlocks(detail.transcript?.utterances ?? []), [detail.transcript]);
  const { active, refs, follow, toggleFollow, showResume, resume } = usePlaybackFollow(audio, blocks);
  const [running, setRunning] = useState(false);
  const [error, setError] = useState<string | null>(null);
  if (!detail.transcript) {
    const preview = async () => {
      setRunning(true);
      setError(null);
      try {
        await api.previewTranscribe(detail.meta.id);
        onReload();
      } catch (e) {
        setError(String(e));
      } finally {
        setRunning(false);
      }
    };
    return (
      <div className="empty-text">
        <p>아직 전사가 없습니다. Mac의 sori가 이 녹음을 받아 적으면 여기에 나타납니다.</p>
        {detail.audioAvailable && (
          <button className="secondary-button" onClick={preview} disabled={running}>
            {running ? "iPhone에서 전사 중. 1시간 녹음이면 몇 분 걸립니다" : "iPhone에서 바로 전사(화자 없이)"}
          </button>
        )}
        {error && <p className="error">{error}</p>}
      </div>
    );
  }
  if (blocks.length === 0) return <p className="empty-text">받아 적을 말소리가 없었습니다.</p>;
  return (
    <div className="transcript">
      {detail.transcriptPreview && (
        <p className="preview-note muted small">iPhone에서 만든 미리 전사입니다. 화자는 나누지 않았습니다. Mac의 sori가 받아 적으면 화자를 나눈 전사로 바뀝니다.</p>
      )}
      {audio && (
        <button className={`follow-toggle ${follow ? "on" : ""}`} aria-pressed={follow} onClick={toggleFollow}>
          자동 스크롤 {follow ? "켬" : "끔"}
        </button>
      )}
      {blocks.map((b, i) => (
        <BlockView
          key={`${b.start}-${b.speaker}`}
          block={b}
          name={speakerName(detail.meta, b.speaker)}
          active={i === active}
          onSeek={onSeek}
          blockRef={(el) => {
            refs.current[i] = el;
          }}
        />
      ))}
      {showResume && (
        <button className="follow-resume" onClick={resume}>
          지금 위치로
        </button>
      )}
    </div>
  );
}

/** 재생 중 블록이 바뀔 때 바뀐 두 블록만 다시 그린다. 1시간 회의면 블록이 수백 개다 */
const BlockView = memo(
  function BlockView({
    block,
    name,
    active,
    onSeek,
    blockRef,
  }: {
    block: Block;
    name: string;
    active: boolean;
    onSeek: (t: number) => void;
    blockRef: (el: HTMLElement | null) => void;
  }) {
    return (
      <section ref={blockRef} className={`block ${active ? "active" : ""}`}>
        <button className="block-head" onClick={() => onSeek(block.start)}>
          <span className="speaker" style={{ color: `var(--spk-${block.speaker % 8})` }}>
            {name}
          </span>
          <span className="muted">{formatTime(block.start)}</span>
        </button>
        <p>{block.utterances.map((u) => u.text.trim()).join(" ")}</p>
      </section>
    );
  },
  // blockRef, onSeek은 그릴 때마다 새로 만들어지지만 하는 일은 같으므로 비교하지 않는다
  (a, b) => a.block === b.block && a.name === b.name && a.active === b.active,
);

function SummaryView({ detail }: { detail: RecordingDetail }) {
  const [index, setIndex] = useState(detail.summaries.length - 1);
  if (detail.summaries.length === 0) {
    return <p className="empty-text">아직 요약이 없습니다. Mac의 sori에서 요약하면 여기에 나타납니다.</p>;
  }
  const summary = detail.summaries[Math.min(index, detail.summaries.length - 1)];
  return (
    <div className="summary">
      {detail.summaries.length > 1 && (
        <select value={index} onChange={(e) => setIndex(Number(e.target.value))} aria-label="요약 고르기">
          {detail.summaries.map((s, i) => (
            <option key={s.id ?? i} value={i}>
              {s.template ?? "요약"} ({new Date(s.createdAt).toLocaleDateString("ko-KR")})
            </option>
          ))}
        </select>
      )}
      <div className="markdown">
        {/* 요약은 LLM이 쓴 글이라 이미지 주소로 바깥에 요청을 보내지 않게 막는다 */}
        <Markdown remarkPlugins={[remarkGfm]} disallowedElements={["img"]}>
          {summary.markdown}
        </Markdown>
      </div>
    </div>
  );
}

function NotesView({
  detail,
  onSeek,
  currentTime,
  onError,
}: {
  detail: RecordingDetail;
  onSeek: (t: number) => void;
  currentTime: () => number;
  onError: (e: string) => void;
}) {
  const [text, setText] = useState("");
  const notes = detail.meta.notes ?? [];
  const add = async () => {
    if (!text.trim()) return;
    try {
      await api.addNote(detail.meta.id, currentTime(), text);
      setText("");
    } catch (e) {
      onError(String(e));
    }
  };
  return (
    <div className="notes">
      <form
        className="note-form"
        onSubmit={(e) => {
          e.preventDefault();
          add();
        }}
      >
        <input value={text} onChange={(e) => setText(e.target.value)} placeholder="재생 위치에 메모" enterKeyHint="done" />
        <button type="submit" disabled={!text.trim()}>
          추가
        </button>
      </form>
      {notes.length === 0 && <p className="empty-text">메모가 없습니다.</p>}
      <ul className="note-list">
        {notes.map((n) => (
          <li key={n.id ?? `${n.time}-${n.text}`}>
            <button className="note-time" onClick={() => onSeek(n.time)}>
              {formatTime(n.time)}
            </button>
            <span className="note-text">{n.text || "북마크"}</span>
            {n.id && (
              <button
                className="text-button danger small"
                aria-label="메모 지우기"
                onClick={() => api.deleteNote(detail.meta.id, n.id!).catch((e) => onError(String(e)))}
              >
                지우기
              </button>
            )}
          </li>
        ))}
      </ul>
    </div>
  );
}

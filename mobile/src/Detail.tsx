import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { convertFileSrc } from "@tauri-apps/api/core";
import Markdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { formatBytes, formatTime } from "../../src/api";
import { toBlocks } from "../../src/markdown";
import { api, Recording, RecordingDetail } from "./api";

type Tab = "transcript" | "summary" | "notes";

function speakerName(meta: Recording, speaker: number): string {
  return meta.speakerNames?.[String(speaker)] ?? `화자 ${speaker + 1}`;
}

export function Detail({ id, onBack }: { id: string; onBack: () => void }) {
  const [detail, setDetail] = useState<RecordingDetail | null>(null);
  const [tab, setTab] = useState<Tab>("transcript");
  const [error, setError] = useState<string | null>(null);
  const audio = useRef<HTMLAudioElement>(null);

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
          <span>{new Date(meta.createdAt).toLocaleString("ko-KR", { dateStyle: "medium", timeStyle: "short" })}</span>
          {meta.duration != null && <span>{formatTime(meta.duration)}</span>}
        </p>
        {error && <p className="error">{error}</p>}

        {detail.audioAvailable ? (
          <audio ref={audio} className="player" controls preload="metadata" src={convertFileSrc(detail.audioPath)} />
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

        {tab === "transcript" && <TranscriptView detail={detail} onSeek={seek} />}
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

function TranscriptView({ detail, onSeek }: { detail: RecordingDetail; onSeek: (t: number) => void }) {
  const blocks = useMemo(() => toBlocks(detail.transcript?.utterances ?? []), [detail.transcript]);
  if (!detail.transcript) {
    return <p className="empty-text">아직 전사가 없습니다. Mac의 sori가 이 녹음을 받아 적으면 여기에 나타납니다.</p>;
  }
  if (blocks.length === 0) return <p className="empty-text">받아 적을 말소리가 없었습니다.</p>;
  return (
    <div className="transcript">
      {blocks.map((b) => (
        <section key={`${b.start}-${b.speaker}`} className="block">
          <button className="block-head" onClick={() => onSeek(b.start)}>
            <span className="speaker" style={{ color: `var(--spk-${b.speaker % 8})` }}>
              {speakerName(detail.meta, b.speaker)}
            </span>
            <span className="muted">{formatTime(b.start)}</span>
          </button>
          <p>{b.utterances.map((u) => u.text.trim()).join(" ")}</p>
        </section>
      ))}
    </div>
  );
}

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

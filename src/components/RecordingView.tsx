import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { convertFileSrc } from "@tauri-apps/api/core";
import { SummaryPanel } from "./SummaryPanel";
import { Block, TranscriptList } from "./TranscriptList";
import { api, formatTime, LANGUAGES, ProgressEvent, Recording, RecordingDetail, speakerName, Stage, Utterance } from "../api";

const STAGE_LABEL: Record<Stage, string> = {
  decoding: "오디오 읽는 중",
  diarizing: "화자 나누는 중",
  transcribing: "받아 적는 중",
};

interface Props {
  recording: Recording;
  progress?: ProgressEvent;
  onChanged: () => Promise<void>;
  onDeleted: () => Promise<void>;
}

/** 같은 화자가 이어서 말한 발화를 한 덩어리로 보여 준다 */
function toBlocks(utterances: Utterance[]): Block[] {
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

export function RecordingView({ recording, progress, onChanged, onDeleted }: Props) {
  const [detail, setDetail] = useState<RecordingDetail | null>(null);
  const [numSpeakers, setNumSpeakers] = useState<string>(recording.numSpeakers?.toString() ?? "");
  const [languages, setLanguages] = useState<string[]>(recording.languages);
  const [running, setRunning] = useState(recording.status === "processing");
  const [error, setError] = useState<string | null>(recording.error);
  const [currentTime, setCurrentTime] = useState(0);
  const [playing, setPlaying] = useState(false);
  const [editingSpeaker, setEditingSpeaker] = useState<number | null>(null);
  const [copied, setCopied] = useState(false);
  const [tab, setTab] = useState<"transcript" | "summary">("transcript");
  const audioRef = useRef<HTMLAudioElement>(null);

  const load = useCallback(async () => {
    const d = await api.getRecording(recording.id);
    setDetail(d);
    if (d.recording.languages.length === 0 && d.transcript === null) {
      const settings = await api.getSettings();
      setLanguages(settings.defaultLanguages);
    }
  }, [recording.id]);

  useEffect(() => {
    load();
  }, [load]);

  const rec = detail?.recording ?? recording;
  const transcript = detail?.transcript ?? null;
  const blocks = useMemo(() => toBlocks(transcript?.utterances ?? []), [transcript]);
  const speakers = useMemo(
    () => [...new Set((transcript?.utterances ?? []).map((u) => u.speaker))].sort((a, b) => a - b),
    [transcript],
  );

  const start = async () => {
    setRunning(true);
    setError(null);
    const n = numSpeakers.trim() === "" ? null : Number(numSpeakers);
    try {
      const pending = api.processRecording(rec.id, n, languages);
      await onChanged();
      await pending;
    } catch (e) {
      setError(String(e));
    } finally {
      setRunning(false);
      await load();
      await onChanged();
    }
  };

  const saveRecording = async (next: Recording) => {
    await api.updateRecording(next);
    await load();
    await onChanged();
  };

  const remove = async () => {
    if (!confirm(`"${rec.title}" 녹음과 전사 결과를 삭제할까요? 되돌릴 수 없습니다.`)) return;
    try {
      await api.deleteRecording(rec.id);
      await onDeleted();
    } catch (e) {
      setError(String(e));
    }
  };

  // 블록 컴포넌트가 memo로 다시 그려지지 않도록 같은 함수를 넘긴다
  const seek = useCallback((time: number) => {
    const audio = audioRef.current;
    if (!audio) return;
    audio.currentTime = time;
    audio.play();
  }, []);

  const copyText = async () => {
    const text = blocks
      .map((b) => `[${formatTime(b.start)}] ${speakerName(rec, b.speaker)}: ${b.utterances.map((u) => u.text).join(" ")}`)
      .join("\n\n");
    await navigator.clipboard.writeText(text);
    setCopied(true);
    setTimeout(() => setCopied(false), 1500);
  };

  const toggleLanguage = (code: string) =>
    setLanguages((prev) => (prev.includes(code) ? prev.filter((l) => l !== code) : [...prev, code]));

  return (
    <div className="recording-view">
      <header className="recording-header">
        <input
          className="title-input"
          defaultValue={rec.title}
          aria-label="녹음 제목"
          onBlur={(e) => {
            const title = e.target.value.trim();
            if (title && title !== rec.title) saveRecording({ ...rec, title });
          }}
        />
        <div className="header-actions">
          {transcript && (
            <button onClick={copyText}>{copied ? "복사했습니다" : "텍스트 복사"}</button>
          )}
          <button className="danger" onClick={remove} disabled={running || rec.status === "recording"}>
            삭제
          </button>
        </div>
      </header>

      {rec.status === "recording" && (
        <p className="recording-notice">녹음 중입니다. 왼쪽 위에서 멈추면 재생하고 받아 적을 수 있습니다.</p>
      )}

      {detail && rec.status !== "recording" && (
        <audio
          ref={audioRef}
          className="player"
          controls
          src={convertFileSrc(detail.audioPath)}
          onTimeUpdate={(e) => setCurrentTime(e.currentTarget.currentTime)}
          onPlay={() => setPlaying(true)}
          onPause={() => setPlaying(false)}
        />
      )}

      {rec.status !== "recording" && (
      <section className="process-panel">
        <label className="field">
          <span>화자 수</span>
          <input
            type="number"
            min={1}
            max={20}
            placeholder="자동"
            value={numSpeakers}
            disabled={running}
            onChange={(e) => setNumSpeakers(e.target.value)}
          />
        </label>
        <div className="field">
          <span>녹음 언어</span>
          <div className="chips">
            {LANGUAGES.map((l) => (
              <button
                key={l.code}
                className={`chip ${languages.includes(l.code) ? "on" : ""}`}
                disabled={running}
                aria-pressed={languages.includes(l.code)}
                onClick={() => toggleLanguage(l.code)}
              >
                {l.label}
              </button>
            ))}
          </div>
        </div>
        <div className="process-actions">
          {running ? (
            <button onClick={() => api.cancelProcessing(rec.id)}>취소</button>
          ) : (
            <button className="primary" onClick={start}>
              {transcript ? "다시 처리" : "전사 시작"}
            </button>
          )}
        </div>
        <p className="hint muted">
          화자 수를 알면 꼭 적어 주세요. 자동 추정은 회의실처럼 울리는 녹음에서 화자를 지나치게 많이 나눕니다. 언어를 고르지 않으면 구간마다 자동 감지합니다.
        </p>
      </section>
      )}

      {running && (
        <div className="progress" aria-live="polite">
          <div className="progress-label">
            {progress ? `${STAGE_LABEL[progress.stage]} ${Math.round(progress.progress * 100)}%` : "준비 중"}
          </div>
          <div className="progress-track">
            <div className="progress-bar" style={{ width: `${(progress?.progress ?? 0) * 100}%` }} />
          </div>
        </div>
      )}

      {error && !running && <div className="error">{error}</div>}

      {transcript && (
        <div className="tabs" role="tablist">
          <button role="tab" aria-selected={tab === "transcript"} className={tab === "transcript" ? "on" : ""} onClick={() => setTab("transcript")}>
            전사
          </button>
          <button role="tab" aria-selected={tab === "summary"} className={tab === "summary" ? "on" : ""} onClick={() => setTab("summary")}>
            요약{detail?.summary ? "" : " (없음)"}
          </button>
        </div>
      )}

      {transcript && tab === "summary" && (
        <div className="tab-scroll">
          <SummaryPanel recordingId={rec.id} summary={detail?.summary ?? null} onSummarized={load} />
        </div>
      )}

      {transcript && tab === "transcript" && speakers.length > 0 && (
        <section className="speakers">
          {speakers.map((s) =>
            editingSpeaker === s ? (
              <input
                key={s}
                autoFocus
                className={`speaker-input spk-${s % 8}`}
                defaultValue={speakerName(rec, s)}
                onBlur={(e) => {
                  setEditingSpeaker(null);
                  const name = e.target.value.trim();
                  const speakerNames = { ...rec.speakerNames };
                  if (name) speakerNames[String(s)] = name;
                  else delete speakerNames[String(s)];
                  saveRecording({ ...rec, speakerNames });
                }}
                onKeyDown={(e) => e.key === "Enter" && e.currentTarget.blur()}
              />
            ) : (
              <button key={s} className={`speaker-tag spk-${s % 8}`} onClick={() => setEditingSpeaker(s)} title="이름 바꾸기">
                {speakerName(rec, s)}
                {transcript.origins?.[String(s)] && (
                  <span className="origin">{transcript.origins[String(s)] === "local" ? "마이크" : "원격"}</span>
                )}
              </button>
            ),
          )}
        </section>
      )}

      {transcript && tab === "transcript" && (
        <TranscriptList blocks={blocks} recording={rec} currentTime={currentTime} playing={playing} onSeek={seek} />
      )}
    </div>
  );
}

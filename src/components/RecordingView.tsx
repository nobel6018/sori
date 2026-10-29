import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { convertFileSrc } from "@tauri-apps/api/core";
import { AskPanel } from "./AskPanel";
import { NotesPanel } from "./NotesPanel";
import { SummaryPanel } from "./SummaryPanel";
import { TranscriptList } from "./TranscriptList";
import { api, LANGUAGES, ProgressEvent, Recording, RecordingDetail, speakerName, Stage } from "../api";
import { EXPORT_LABEL, ExportFormat, exportBytes, exportFileName } from "../export";
import { recordingMarkdown, toBlocks, transcriptMarkdown } from "../markdown";
import { IS_MAC, useCommands } from "../shortcuts";

const STAGE_LABEL: Record<Stage, string> = {
  waiting: "앞 녹음의 처리가 끝나길 기다리는 중. 한 번에 하나씩 처리합니다",
  decoding: "오디오 읽는 중",
  diarizing: "화자 나누는 중",
  transcribing: "받아 적는 중",
};

interface Props {
  recording: Recording;
  progress?: ProgressEvent;
  /** 검색 결과나 링크로 열었을 때 옮겨 갈 재생 위치 */
  seekTo: { time: number; nonce: number } | null;
  /** 재생 위치를 옮겼다. 다시 그려질 때 또 옮기지 않도록 App이 지운다 */
  onSeekDone: () => void;
  onChanged: () => Promise<void>;
  onDeleted: () => Promise<void>;
}

export function RecordingView({ recording, progress, seekTo, onSeekDone, onChanged, onDeleted }: Props) {
  const [detail, setDetail] = useState<RecordingDetail | null>(null);
  const [numSpeakers, setNumSpeakers] = useState<string>(recording.numSpeakers?.toString() ?? "");
  const [languages, setLanguages] = useState<string[]>(recording.languages);
  const [diarize, setDiarize] = useState(recording.diarize ?? true);
  const [running, setRunning] = useState(recording.status === "processing");
  const [error, setError] = useState<string | null>(recording.error);
  const [currentTime, setCurrentTime] = useState(0);
  const [playing, setPlaying] = useState(false);
  const [follow, setFollow] = useState(() => {
    try {
      return localStorage.getItem("sori.follow") !== "off";
    } catch {
      return true;
    }
  });
  const toggleFollow = () =>
    setFollow((prev) => {
      try {
        localStorage.setItem("sori.follow", prev ? "off" : "on");
      } catch {
        // 저장이 안 되면 이번 실행 동안만 유지한다
      }
      return !prev;
    });
  const [editingSpeaker, setEditingSpeaker] = useState<number | null>(null);
  const [copied, setCopied] = useState<"text" | "markdown" | null>(null);
  const [tab, setTab] = useState<"transcript" | "summary" | "ask" | "notes">("transcript");
  const audioRef = useRef<HTMLAudioElement>(null);

  const load = useCallback(async () => {
    const d = await api.getRecording(recording.id);
    setDetail(d);
    if (d.transcript === null) {
      // 아직 처리하지 않은 녹음은 설정의 기본값을 미리 고른다
      const settings = await api.getSettings();
      if (d.recording.languages.length === 0) setLanguages(settings.defaultLanguages);
      setDiarize(settings.defaultDiarize);
    }
  }, [recording.id]);

  useEffect(() => {
    load();
  }, [load]);

  // 전역 북마크 단축키처럼 화면 밖에서 메모가 늘면 다시 읽는다
  useEffect(() => {
    const unlisten = api.onRecordingChanged((id) => id === recording.id && load());
    return () => {
      unlisten.then((f) => f());
    };
  }, [recording.id, load]);

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
      const pending = api.processRecording(rec.id, diarize ? n : null, languages, diarize);
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

  const nameSpeaker = async (speaker: number, name: string) => {
    try {
      await api.nameSpeaker(rec.id, speaker, name);
    } catch (e) {
      setError(String(e));
    }
    await load();
    await onChanged();
  };

  const exportAs = async (format: ExportFormat) => {
    if (!detail) return;
    try {
      await api.saveExport(exportFileName(rec.title, format), await exportBytes(detail, format));
    } catch (e) {
      setError(String(e));
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

  const copy = async (what: "text" | "markdown") => {
    if (!detail?.transcript) return;
    const text = what === "markdown" ? recordingMarkdown(detail) : transcriptMarkdown(rec, detail.transcript.utterances);
    await navigator.clipboard.writeText(text);
    setCopied(what);
    setTimeout(() => setCopied(null), 1500);
  };

  // 검색 결과에서 고른 문장으로 옮겨 간다. 재생은 사용자가 시작한다
  const [scrollTarget, setScrollTarget] = useState<{ time: number; nonce: number } | null>(null);
  useEffect(() => {
    const audio = audioRef.current;
    if (!seekTo || !audio) return;
    const apply = () => {
      audio.currentTime = seekTo.time;
      setCurrentTime(seekTo.time);
      setTab("transcript");
      setScrollTarget(seekTo);
      onSeekDone();
    };
    if (audio.readyState >= 1) apply();
    else {
      audio.addEventListener("loadedmetadata", apply, { once: true });
      return () => audio.removeEventListener("loadedmetadata", apply);
    }
    // detail이 오면 재생기가 생기므로 그때 다시 본다
  }, [seekTo, detail, onSeekDone]);

  const togglePlay = () => {
    const audio = audioRef.current;
    if (!audio) return;
    if (audio.paused) audio.play();
    else audio.pause();
  };

  const skip = (seconds: number) => {
    const audio = audioRef.current;
    if (!audio) return;
    audio.currentTime = Math.min(Math.max(0, audio.currentTime + seconds), audio.duration || Infinity);
  };

  useCommands((command) => {
    switch (command) {
      case "process":
        if (!running && rec.status !== "recording") start();
        return;
      case "cancel":
        if (running) api.cancelProcessing(rec.id);
        return;
      case "tab-transcript":
        if (transcript) setTab("transcript");
        return;
      case "tab-summary":
        if (transcript) setTab("summary");
        return;
      case "summarize":
        // 요약은 요약 패널이 같은 명령을 받아 시작한다. 여기서는 탭만 연다
        if (transcript) setTab("summary");
        return;
      case "play-pause":
        return togglePlay();
      case "seek-back":
        return skip(-5);
      case "seek-forward":
        return skip(5);
      case "toggle-follow":
        return toggleFollow();
      case "copy-markdown":
        return copy("markdown");
      case "delete":
        if (!running && rec.status !== "recording") remove();
        return;
      case "reveal":
        return api.revealAudio(rec.id).catch((e) => setError(String(e)));
      case "bookmark": {
        // 다른 녹음이 진행 중이면 그 녹음에 단다. 회의 중에 예전 녹음을 보다가 누른 경우다
        const playback = () => api.addNote(rec.id, audioRef.current?.currentTime ?? 0, "");
        const pending = rec.status === "recording" ? api.addNote(rec.id, null, "") : api.addNote(null, null, "").catch(playback);
        return pending.then(load).catch((e) => setError(String(e)));
      }
      case "tab-ask":
        if (transcript) setTab("ask");
        return;
      case "tab-notes":
        return setTab("notes");
    }
  });

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
          <button onClick={() => api.revealAudio(rec.id).catch((e) => setError(String(e)))} title={IS_MAC ? "⇧⌘E" : "Ctrl+Shift+E"}>
            {IS_MAC ? "Finder에서 보기" : "탐색기에서 보기"}
          </button>
          {transcript && (
            <>
              <select
                className="export-select"
                aria-label="내보내기"
                value=""
                onChange={(e) => e.target.value && exportAs(e.target.value as ExportFormat)}
              >
                <option value="">내보내기</option>
                {(Object.keys(EXPORT_LABEL) as ExportFormat[]).map((f) => (
                  <option key={f} value={f}>
                    {EXPORT_LABEL[f]}
                  </option>
                ))}
              </select>
              <button onClick={() => copy("text")}>{copied === "text" ? "복사했습니다" : "전사 복사"}</button>
              <button onClick={() => copy("markdown")} title={IS_MAC ? "⇧⌘C" : "Ctrl+Shift+C"}>
                {copied === "markdown" ? "복사했습니다" : "마크다운 복사"}
              </button>
            </>
          )}
          <button className="danger" onClick={remove} disabled={running || rec.status === "recording"}>
            삭제
          </button>
        </div>
      </header>

      {rec.status === "recording" && (
        <>
          <p className="recording-notice">녹음 중입니다. 왼쪽 위에서 멈추면 재생하고 받아 적을 수 있습니다.</p>
          <NotesPanel recording={rec} recordingNow currentTime={0} onSeek={seek} onChanged={load} />
        </>
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
        <div className="field">
          <span>처리 방식</span>
          <div className="segmented" role="radiogroup" aria-label="처리 방식">
            <button role="radio" aria-checked={diarize} className={diarize ? "on" : ""} disabled={running} onClick={() => setDiarize(true)}>
              전사 + 화자 나누기
            </button>
            <button role="radio" aria-checked={!diarize} className={!diarize ? "on" : ""} disabled={running} onClick={() => setDiarize(false)}>
              전사만
            </button>
          </div>
        </div>
        {diarize && (
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
        )}
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
          {diarize
            ? "화자 수를 알면 적어 주세요. 자동 추정은 시험한 녹음 6개 중 5개를 맞혔지만, 목소리가 비슷하거나 울리는 방에서는 틀릴 수 있습니다. "
            : "전사만 하면 화자를 나누지 않아 4배쯤 빠릅니다. 혼자 말한 메모나 강의에 맞습니다. 여러 언어가 섞인 녹음은 언어가 섞인 채 옮겨지니 화자 나누기를 고르세요. "}
          언어를 고르지 않으면 구간마다 자동 감지합니다.
        </p>
      </section>
      )}

      {running && (
        <div className="progress" aria-live="polite">
          <div className="progress-label">
            {!progress
              ? "준비 중"
              : progress.stage === "waiting"
                ? STAGE_LABEL.waiting
                : `${STAGE_LABEL[progress.stage]} ${Math.round(progress.progress * 100)}%`}
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
            요약{detail?.summaries.length ? (detail.summaries.length > 1 ? ` ${detail.summaries.length}` : "") : " (없음)"}
          </button>
          <button role="tab" aria-selected={tab === "ask"} className={tab === "ask" ? "on" : ""} onClick={() => setTab("ask")}>
            질문
          </button>
          <button role="tab" aria-selected={tab === "notes"} className={tab === "notes" ? "on" : ""} onClick={() => setTab("notes")}>
            메모{rec.notes.length ? ` ${rec.notes.length}` : ""}
          </button>
        </div>
      )}

      {/* 탭을 바꿔도 요약, 질문 진행 상태를 잃지 않도록 숨기기만 한다 */}
      {transcript && (
        <div className="tab-scroll" hidden={tab !== "summary"}>
          <SummaryPanel recordingId={rec.id} summaries={detail?.summaries ?? []} onSummarized={load} />
        </div>
      )}
      {transcript && (
        <div className="tab-scroll" hidden={tab !== "ask"}>
          <AskPanel
            recordingId={rec.id}
            onCite={(_, time) => {
              if (time == null) return;
              setTab("transcript");
              setScrollTarget({ time, nonce: Date.now() });
              seek(time);
            }}
          />
        </div>
      )}
      {transcript && tab === "notes" && (
        <div className="tab-scroll">
          <NotesPanel recording={rec} recordingNow={false} currentTime={currentTime} onSeek={seek} onChanged={load} />
        </div>
      )}

      {transcript && tab === "transcript" && speakers.length > 0 && (
        <section className="speakers">
          <button
            className={`follow-toggle ${follow ? "on" : ""}`}
            aria-pressed={follow}
            title="재생 중 지금 말하는 부분으로 전사를 자동 스크롤합니다"
            onClick={toggleFollow}
          >
            자동 스크롤 {follow ? "켬" : "끔"}
          </button>
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
                  if (name !== (rec.speakerNames[String(s)] ?? "")) nameSpeaker(s, name);
                }}
                onKeyDown={(e) => e.key === "Enter" && e.currentTarget.blur()}
              />
            ) : (
              <button
                key={s}
                className={`speaker-tag spk-${s % 8}`}
                onClick={() => setEditingSpeaker(s)}
                title={rec.voiceIds?.[String(s)] ? "기억한 목소리로 알아본 이름입니다. 눌러서 바꾸면 기억도 고칩니다" : "이름 바꾸기. 바꾼 이름은 목소리와 함께 기억합니다"}
              >
                {speakerName(rec, s)}
                {rec.voiceIds?.[String(s)] && <span className="origin">목소리</span>}
                {transcript.origins?.[String(s)] && (
                  <span className="origin">{transcript.origins[String(s)] === "local" ? "마이크" : "원격"}</span>
                )}
              </button>
            ),
          )}
        </section>
      )}

      {transcript && tab === "transcript" && (
        <TranscriptList
          blocks={blocks}
          recording={rec}
          currentTime={currentTime}
          follow={playing && follow}
          scrollTarget={scrollTarget}
          notes={rec.notes}
          onSeek={seek}
        />
      )}
    </div>
  );
}

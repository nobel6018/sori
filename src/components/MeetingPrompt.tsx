import { useEffect, useState } from "react";
import { api, MeetingPrompt as Prompt } from "../api";

/** 이 시간 안에 답하지 않으면 묻지 않은 것으로 하고 닫는다 */
const TIMEOUT_MS = 30_000;

/**
 * 회의 앱이 마이크를 쓰기 시작하면 화면 오른쪽 위에 뜨는 작은 창.
 * 회의 앱을 보는 중이라 초점을 가져가지 않고, 30초 뒤 저절로 닫힌다
 */
export function MeetingPrompt() {
  const [prompt, setPrompt] = useState<Prompt | null>(null);

  useEffect(() => {
    api.meetingPrompt().then(setPrompt);
    const timer = setTimeout(() => api.meetingPromptRespond(false), TIMEOUT_MS);
    return () => clearTimeout(timer);
  }, []);

  if (!prompt) return null;
  const start = prompt.kind === "start";
  return (
    <div className="meeting-prompt" data-tauri-drag-region>
      <img src="/favicon.svg" width="28" height="28" alt="" />
      <div className="meeting-text" data-tauri-drag-region>
        <strong>{start ? `${prompt.app}에서 마이크를 쓰기 시작했습니다` : `${prompt.app} 통화가 끝난 것 같습니다`}</strong>
        <span>{start ? "sori로 녹음할까요?" : "녹음을 멈출까요?"}</span>
      </div>
      <div className="meeting-actions">
        <button onClick={() => api.meetingPromptRespond(false)}>{start ? "괜찮아요" : "계속 녹음"}</button>
        <button className="primary" onClick={() => api.meetingPromptRespond(true)}>
          {start ? "녹음" : "멈추기"}
        </button>
      </div>
    </div>
  );
}

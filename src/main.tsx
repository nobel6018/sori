import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { MeetingPrompt } from "./components/MeetingPrompt";

// 회의 앱 감지 창도 같은 화면 코드를 쓴다. 주소의 view로 무엇을 그릴지 고른다
const view = new URLSearchParams(window.location.search).get("view");

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>{view === "meeting" ? <MeetingPrompt /> : <App />}</React.StrictMode>,
);

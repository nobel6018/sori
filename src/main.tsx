import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { MeetingPrompt } from "./components/MeetingPrompt";

// 회의 앱 감지 창도 같은 화면 코드를 쓴다. 주소의 view로 무엇을 그릴지 고른다
// 웹뷰 기본 우클릭 메뉴는 "다시 불러오기"뿐이라 앱에서는 쓸모가 없고, 누르면 진행 중인 화면 상태가 날아간다.
// 글 입력 칸과 선택한 글(복사, 찾아보기)에서는 그대로 둔다. 개발 빌드는 요소 검사를 쓰려고 남긴다
// @see docs/context-menu.md
if (import.meta.env.PROD) {
  window.addEventListener("contextmenu", (e) => {
    const target = e.target as HTMLElement | null;
    const editable = target?.closest("input, textarea, [contenteditable='true']");
    const selected = !!window.getSelection()?.toString();
    if (!editable && !selected) e.preventDefault();
  });
}

const view = new URLSearchParams(window.location.search).get("view");

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>{view === "meeting" ? <MeetingPrompt /> : <App />}</React.StrictMode>,
);

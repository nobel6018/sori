import { useEffect, useRef } from "react";

// 앱 안 단축키와, 단축키가 화면 곳곳의 컴포넌트에 일을 시키는 통로.
// @see docs/shortcuts-and-deep-links.md

export const IS_MAC = navigator.userAgent.includes("Mac");

/** 단축키나 링크가 컴포넌트에 시키는 일. 녹음기와 녹음 화면이 각자 구독한다 */
export type Command =
  | "toggle-recording"
  | "start-recording"
  | "stop-recording"
  | "process"
  | "cancel"
  | "summarize"
  | "tab-transcript"
  | "tab-summary"
  | "play-pause"
  | "seek-back"
  | "seek-forward"
  | "toggle-follow"
  | "copy-markdown"
  | "delete"
  | "reveal";

const EVENT = "sori:command";

export function sendCommand(command: Command) {
  window.dispatchEvent(new CustomEvent<Command>(EVENT, { detail: command }));
}

/** 명령을 받는다. handler가 바뀌어도 다시 구독하지 않도록 최신 함수를 ref로 들고 있는다 */
export function useCommands(handler: (command: Command) => void) {
  const ref = useRef(handler);
  ref.current = handler;
  useEffect(() => {
    const listener = (e: Event) => ref.current((e as CustomEvent<Command>).detail);
    window.addEventListener(EVENT, listener);
    return () => window.removeEventListener(EVENT, listener);
  }, []);
}

/** App이 직접 처리하는 일 */
export type AppCommand = "import" | "settings" | "search" | "help" | "prev" | "next";

export interface Shortcut {
  /** 키 이름(KeyboardEvent.key 기준, 글자는 소문자) */
  key: string;
  /** ⌘(Windows는 Ctrl) */
  mod?: boolean;
  shift?: boolean;
  run: Command | AppCommand;
  label: string;
  group: "파일" | "처리" | "이동" | "재생" | "보기";
  /** 글자 입력칸에 커서가 있어도 동작하는지. 입력칸의 기본 동작(⌘↑, ⌘⌫ 등)과 겹치면 false */
  inInput?: boolean;
}

export const SHORTCUTS: Shortcut[] = [
  { key: "o", mod: true, run: "import", label: "오디오 가져오기", group: "파일", inInput: true },
  { key: ",", mod: true, run: "settings", label: "설정 열기", group: "파일", inInput: true },
  { key: "Backspace", mod: true, run: "delete", label: "녹음 지우기", group: "파일" },
  { key: "e", mod: true, shift: true, run: "reveal", label: "오디오 파일을 Finder에서 보기", group: "파일" },
  { key: "Enter", mod: true, run: "process", label: "전사 시작, 다시 처리", group: "처리", inInput: true },
  { key: "Enter", mod: true, shift: true, run: "summarize", label: "요약 만들기", group: "처리", inInput: true },
  { key: "Escape", run: "cancel", label: "처리나 요약 취소", group: "처리" },
  { key: "k", mod: true, run: "search", label: "빠른 찾기 (미리 보기, 복사)", group: "이동", inInput: true },
  { key: "f", mod: true, run: "search", label: "전사 검색", group: "이동", inInput: true },
  { key: "ArrowUp", mod: true, run: "prev", label: "이전 녹음", group: "이동" },
  { key: "ArrowDown", mod: true, run: "next", label: "다음 녹음", group: "이동" },
  { key: "1", mod: true, run: "tab-transcript", label: "전사 탭", group: "이동" },
  { key: "2", mod: true, run: "tab-summary", label: "요약 탭", group: "이동" },
  { key: " ", run: "play-pause", label: "재생, 일시정지", group: "재생" },
  { key: "ArrowLeft", run: "seek-back", label: "5초 뒤로", group: "재생" },
  { key: "ArrowRight", run: "seek-forward", label: "5초 앞으로", group: "재생" },
  { key: "l", mod: true, run: "toggle-follow", label: "자동 스크롤 켜고 끄기", group: "보기" },
  { key: "c", mod: true, shift: true, run: "copy-markdown", label: "요약과 전사를 마크다운으로 복사", group: "보기" },
  { key: "/", mod: true, run: "help", label: "단축키 목록", group: "보기", inInput: true },
];

export function isEditable(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  if (target.isContentEditable) return true;
  if (target instanceof HTMLTextAreaElement || target instanceof HTMLSelectElement) return true;
  if (target instanceof HTMLInputElement) return !["button", "checkbox", "radio", "range"].includes(target.type);
  return false;
}

export function matchShortcut(e: KeyboardEvent): Shortcut | undefined {
  const mod = IS_MAC ? e.metaKey : e.ctrlKey;
  // macOS에서 Ctrl, Windows에서 Win 키가 섞이면 다른 조합이다
  const other = IS_MAC ? e.ctrlKey : e.metaKey;
  if (other || e.altKey) return undefined;
  const key = e.key.length === 1 ? e.key.toLowerCase() : e.key;
  const editable = isEditable(e.target);
  return SHORTCUTS.find(
    (s) => s.key === key && !!s.mod === mod && !!s.shift === e.shiftKey && (!editable || s.inInput),
  );
}

const KEY_LABEL: Record<string, string> = {
  Enter: "↩",
  Escape: "Esc",
  Backspace: "⌫",
  ArrowUp: "↑",
  ArrowDown: "↓",
  ArrowLeft: "←",
  ArrowRight: "→",
  " ": "Space",
};

export function shortcutLabel(s: Shortcut): string {
  const key = KEY_LABEL[s.key] ?? s.key.toUpperCase();
  if (IS_MAC) return `${s.shift ? "⇧" : ""}${s.mod ? "⌘" : ""}${key}`;
  return [s.mod && "Ctrl", s.shift && "Shift", key].filter(Boolean).join("+");
}

/** Tauri 단축키 표기를 이 OS의 표기로 */
export function acceleratorLabel(accelerator: string): string {
  if (!accelerator) return "끔";
  const parts = accelerator.split("+");
  const raw = parts.pop() ?? "";
  const named: Record<string, string> = {
    Space: "Space", ArrowUp: "↑", ArrowDown: "↓", ArrowLeft: "←", ArrowRight: "→", Enter: "↩",
    Backquote: "`", Minus: "-", Equal: "=", Comma: ",", Period: ".", Slash: "/", Semicolon: ";",
    Quote: "'", BracketLeft: "[", BracketRight: "]", Backslash: "\\",
  };
  const key = named[raw] ?? raw.replace(/^Key(?=[A-Z]$)/, "").replace(/^Digit(?=[0-9]$)/, "");
  if (IS_MAC) {
    const symbol: Record<string, string> = { Control: "⌃", Alt: "⌥", Option: "⌥", Shift: "⇧", CommandOrControl: "⌘", Command: "⌘", Super: "⌘", Cmd: "⌘", CmdOrCtrl: "⌘" };
    const order = ["⌃", "⌥", "⇧", "⌘"];
    const mods = parts.map((p) => symbol[p] ?? p).sort((a, b) => order.indexOf(a) - order.indexOf(b));
    return mods.join("") + key;
  }
  const name: Record<string, string> = { CommandOrControl: "Ctrl", CmdOrCtrl: "Ctrl", Control: "Ctrl", Super: "Win" };
  return [...parts.map((p) => name[p] ?? p), key].join("+");
}

/** 키 입력을 Tauri 단축키 표기로. 수식키만 눌렀거나 수식키 없이 누르면 null */
export function acceleratorFromEvent(e: KeyboardEvent): string | null {
  const mods: string[] = [];
  if (IS_MAC ? e.metaKey : e.ctrlKey) mods.push("CommandOrControl");
  if (IS_MAC && e.ctrlKey) mods.push("Control");
  if (!IS_MAC && e.metaKey) mods.push("Super");
  if (e.altKey) mods.push("Alt");
  if (e.shiftKey) mods.push("Shift");
  // Alt를 누르면 e.key가 특수 문자로 바뀌므로 자판 위치(code)로 읽는다
  const code = e.code;
  let key: string | null = null;
  if (/^Key[A-Z]$/.test(code)) key = code.slice(3);
  else if (/^Digit[0-9]$/.test(code)) key = code.slice(5);
  else if (/^F([1-9]|1[0-9]|2[0-4])$/.test(code)) key = code;
  else if (["Space", "ArrowUp", "ArrowDown", "ArrowLeft", "ArrowRight", "Enter", "Backquote", "Minus", "Equal", "Comma", "Period", "Slash", "Semicolon", "Quote", "BracketLeft", "BracketRight", "Backslash"].includes(code)) key = code;
  // Shift나 Alt만 붙인 조합은 전역으로 등록하면 다른 앱에서 대문자, 특수 문자 입력을 빼앗는다
  const primary = mods.some((m) => m === "CommandOrControl" || m === "Control" || m === "Super");
  if (!key || !primary) return null;
  return [...mods, key].join("+");
}

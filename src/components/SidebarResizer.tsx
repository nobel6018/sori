import { useCallback, useRef, useState } from "react";

const WIDTH_KEY = "sori.sidebarWidth";
export const SIDEBAR_DEFAULT = 280;
const MIN = 200;
const MAX = 560;
// 본문(녹음 화면)이 이보다 좁아지지 않게 사이드바 폭을 묶는다
const MAIN_MIN = 420;
const STEP = 16;

function clamp(width: number) {
  const max = Math.max(MIN, Math.min(MAX, window.innerWidth - MAIN_MIN));
  return Math.round(Math.min(max, Math.max(MIN, width)));
}

/** 사이드바 폭. 끌어서 바꾼 폭은 다음 실행에도 기억한다 */
export function useSidebarWidth() {
  const [width, setWidth] = useState(() => {
    const saved = Number(localStorage.getItem(WIDTH_KEY));
    return saved ? clamp(saved) : SIDEBAR_DEFAULT;
  });
  const update = useCallback((next: number, persist: boolean) => {
    const w = clamp(next);
    setWidth(w);
    if (!persist) return;
    if (w === SIDEBAR_DEFAULT) localStorage.removeItem(WIDTH_KEY);
    else localStorage.setItem(WIDTH_KEY, String(w));
  }, []);
  return [width, update] as const;
}

interface Props {
  width: number;
  onResize: (width: number, persist: boolean) => void;
}

/** @see docs/organize.md
 * 사이드바 오른쪽 경계의 손잡이. 끌면 폭이 바뀌고, 두 번 누르면 기본 폭으로 돌아간다. 키보드는 좌우 화살표 */
export function SidebarResizer({ width, onResize }: Props) {
  const drag = useRef<{ startX: number; startWidth: number; last: number } | null>(null);
  const [dragging, setDragging] = useState(false);

  return (
    <div
      className={`sidebar-resizer ${dragging ? "dragging" : ""}`}
      role="separator"
      aria-orientation="vertical"
      aria-label="사이드바 폭"
      aria-valuenow={width}
      aria-valuemin={MIN}
      aria-valuemax={MAX}
      tabIndex={0}
      title="끌어서 폭 바꾸기. 두 번 누르면 기본 폭"
      style={{ left: width }}
      onPointerDown={(e) => {
        if (e.button !== 0) return;
        e.preventDefault();
        e.currentTarget.setPointerCapture(e.pointerId);
        drag.current = { startX: e.clientX, startWidth: width, last: width };
        setDragging(true);
        // 손잡이 밖으로 빠르게 끌어도 커서와 글자 선택이 흔들리지 않게 문서 전체에 건다
        document.body.classList.add("col-resizing");
      }}
      onPointerMove={(e) => {
        if (!drag.current) return;
        const next = drag.current.startWidth + e.clientX - drag.current.startX;
        drag.current.last = next;
        onResize(next, false);
      }}
      onPointerUp={() => {
        if (!drag.current) return;
        // 끄는 동안은 상태만 바꾸고, 놓을 때 한 번만 저장한다
        onResize(drag.current.last, true);
        drag.current = null;
        setDragging(false);
        document.body.classList.remove("col-resizing");
      }}
      onPointerCancel={() => {
        drag.current = null;
        setDragging(false);
        document.body.classList.remove("col-resizing");
      }}
      onDoubleClick={() => onResize(SIDEBAR_DEFAULT, true)}
      onKeyDown={(e) => {
        if (e.key === "ArrowLeft") onResize(width - STEP, true);
        else if (e.key === "ArrowRight") onResize(width + STEP, true);
        else return;
        e.preventDefault();
      }}
    />
  );
}

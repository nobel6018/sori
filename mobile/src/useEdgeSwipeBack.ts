import { RefObject, useEffect, useRef } from "react";

// 화면 왼쪽 가장자리에서 오른쪽으로 밀면 iOS처럼 화면이 손가락을 따라오고, 아래 깔린 목록이 왼쪽에서 조금 늦게 따라 나온다.
// 충분히 밀면 뒤로 간다
// @see docs/mobile.md "가장자리 스와이프로 뒤로"

/** 이 폭 안에서 시작한 터치만 받는다. 안쪽의 가로 스크롤, 글 선택과 겹치지 않게 좁게 둔다 */
const EDGE = 24;
/** 이만큼 움직여야 가로인지 세로인지 정한다 */
const SLOP = 10;
/** 화면 폭의 이 비율을 넘기거나, 빠르게 튕기면 뒤로 간다 */
const COMMIT_RATIO = 0.35;
const FLICK_SPEED = 0.5; // px/ms
const DURATION = 220;
/** 아래 화면은 폭의 이 비율만큼 왼쪽에 있다가 위 화면을 다 밀면 제자리에 온다(iOS 내비게이션과 같은 시차) */
const PARALLAX = 0.3;
const EASE = "cubic-bezier(0.2, 0.8, 0.2, 1)";

export function useEdgeSwipeBack(
  ref: RefObject<HTMLElement | null>,
  underRef: RefObject<HTMLElement | null>,
  onBack: () => void,
  enabled = true,
) {
  const onBackRef = useRef(onBack);
  onBackRef.current = onBack;

  useEffect(() => {
    const el = ref.current;
    if (!enabled || !el) return;

    let gesture: { x0: number; y0: number; x: number; t: number; speed: number; active: boolean } | null = null;
    let finishing = false;

    const placeUnder = (offset: number, transition: string) => {
      const under = underRef.current;
      if (!under) return;
      under.style.transition = transition;
      under.style.transform = `translateX(${offset}px)`;
    };
    const reset = () => {
      el.style.transition = "";
      el.style.transform = "";
      el.classList.remove("swiping");
      const under = underRef.current;
      if (under) {
        under.style.transition = "";
        under.style.transform = "";
        under.classList.remove("peek");
      }
    };

    const onStart = (e: TouchEvent) => {
      if (finishing || e.touches.length !== 1) return;
      const t = e.touches[0];
      if (t.clientX > EDGE) return;
      gesture = { x0: t.clientX, y0: t.clientY, x: t.clientX, t: e.timeStamp, speed: 0, active: false };
    };

    const onMove = (e: TouchEvent) => {
      if (!gesture) return;
      const t = e.touches[0];
      const dx = t.clientX - gesture.x0;
      const dy = t.clientY - gesture.y0;
      const width = window.innerWidth;
      if (!gesture.active) {
        // 세로로 먼저 움직이면 평소 스크롤로 넘긴다
        if (Math.abs(dy) > SLOP && Math.abs(dy) >= Math.abs(dx)) {
          gesture = null;
          return;
        }
        if (dx < SLOP) return;
        gesture.active = true;
        el.style.transition = "none";
        el.classList.add("swiping");
        underRef.current?.classList.add("peek");
      }
      e.preventDefault();
      const dt = e.timeStamp - gesture.t;
      if (dt > 0) gesture.speed = (t.clientX - gesture.x) / dt;
      gesture.x = t.clientX;
      gesture.t = e.timeStamp;
      const moved = Math.min(Math.max(0, dx), width);
      el.style.transform = `translateX(${moved}px)`;
      placeUnder(-PARALLAX * (width - moved), "none");
    };

    const onEnd = () => {
      const g = gesture;
      gesture = null;
      if (!g?.active) return;
      const width = window.innerWidth;
      const dx = g.x - g.x0;
      const commit = dx > width * COMMIT_RATIO || (g.speed > FLICK_SPEED && dx > 40);
      finishing = true;
      const transition = `transform ${DURATION}ms ${EASE}`;
      el.style.transition = transition;
      el.style.transform = commit ? `translateX(${width}px)` : "translateX(0)";
      placeUnder(commit ? 0 : -PARALLAX * width, transition);
      // transitionend는 화면이 가려지면 안 올 수 있어 시간으로 마무리한다
      window.setTimeout(() => {
        finishing = false;
        if (!commit) return reset();
        // 아래 화면은 이미 제자리(0)다. transform을 남기면 목록으로 돌아간 뒤 그 안의 fixed 요소(녹음 버튼)가
        // 화면 대신 이 층을 기준으로 놓이므로 먼저 지운다. peek는 언마운트할 때 지워진다
        const under = underRef.current;
        if (under) {
          under.style.transition = "";
          under.style.transform = "";
        }
        onBackRef.current();
      }, DURATION);
    };

    window.addEventListener("touchstart", onStart, { passive: true });
    window.addEventListener("touchmove", onMove, { passive: false });
    window.addEventListener("touchend", onEnd);
    window.addEventListener("touchcancel", onEnd);
    return () => {
      window.removeEventListener("touchstart", onStart);
      window.removeEventListener("touchmove", onMove);
      window.removeEventListener("touchend", onEnd);
      window.removeEventListener("touchcancel", onEnd);
      reset();
    };
  }, [ref, underRef, enabled]);
}

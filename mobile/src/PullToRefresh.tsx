import { ReactNode, useEffect, useRef, useState } from "react";
import { api } from "./api";

// iOS 목록의 당겨서 새로고침. 맨 위에서 아래로 당기면 고무줄처럼 버티며 따라오고, 스피너 살이 하나씩 차오른다.
// 기준선을 넘는 순간 가볍게 진동하고, 놓으면 스피너가 돌며 자리를 잡았다가 동기화가 끝나면 올라간다.
// 웹뷰는 문서 전체가 스크롤되고 바운스를 꺼 두었으므로(styles.css overscroll-behavior) 터치를 직접 다룬다.
// @see docs/mobile.md

/** 이만큼 당긴 뒤 놓으면 새로고침한다(화면에 보이는 거리) */
const THRESHOLD = 64;
/** 새로고침하는 동안 스피너가 머무는 높이 */
const HOLD = 52;
/** 고무줄이 버티는 정도. 손가락이 이만큼 내려가면 화면은 그 63%쯤 따라온다 */
const STRETCH = 220;
const SPOKES = 8;
/** 스피너가 너무 빨리 사라지면 새로고침한 느낌이 없다 */
const MIN_SPIN_MS = 700;
const MAX_SPIN_MS = 20_000;

export function PullToRefresh({
  onRefresh,
  busy,
  enabled = true,
  children,
}: {
  onRefresh: () => void;
  busy: boolean;
  /** 목록이 다른 화면 아래에 깔려 있으면 끈다. 문서 스크롤은 위 화면 것이라 그 화면을 당겨도 반응하면 안 된다 */
  enabled?: boolean;
  children: ReactNode;
}) {
  const [pull, setPull] = useState(0);
  const [dragging, setDragging] = useState(false);
  const [refreshing, setRefreshing] = useState(false);
  const start = useRef<{ y: number; x: number } | null>(null);
  const armed = useRef(false);
  const pullRef = useRef(0);
  const refreshingRef = useRef(false);
  const startedAt = useRef(0);
  const sawBusy = useRef(false);
  const onRefreshRef = useRef(onRefresh);
  onRefreshRef.current = onRefresh;
  const enabledRef = useRef(enabled);
  enabledRef.current = enabled;

  useEffect(() => {
    const setP = (v: number) => {
      pullRef.current = v;
      setPull(v);
    };
    const onStart = (e: TouchEvent) => {
      if (!enabledRef.current || refreshingRef.current || window.scrollY > 0 || e.touches.length !== 1) return;
      const t = e.touches[0];
      start.current = { y: t.clientY, x: t.clientX };
      armed.current = false;
    };
    const onMove = (e: TouchEvent) => {
      if (!start.current) return;
      const t = e.touches[0];
      const dy = t.clientY - start.current.y;
      const dx = Math.abs(t.clientX - start.current.x);
      // 위로 밀거나 옆으로 미는 몸짓(태그 줄 가로 스크롤)은 그대로 둔다
      if (dy <= 0 || (pullRef.current === 0 && dx > dy)) {
        if (pullRef.current === 0) start.current = null;
        else setP(0);
        return;
      }
      e.preventDefault();
      setDragging(true);
      const visible = STRETCH * (1 - Math.exp(-dy / STRETCH));
      setP(visible);
      if (!armed.current && visible >= THRESHOLD) {
        armed.current = true;
        api.haptic();
      } else if (armed.current && visible < THRESHOLD) {
        armed.current = false;
      }
    };
    const onEnd = () => {
      if (!start.current) return;
      start.current = null;
      setDragging(false);
      if (armed.current) {
        armed.current = false;
        refreshingRef.current = true;
        sawBusy.current = false;
        startedAt.current = Date.now();
        setRefreshing(true);
        setP(HOLD);
        onRefreshRef.current();
      } else {
        setP(0);
      }
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
    };
  }, []);

  // 동기화가 시작됐다 끝나면(또는 동기화를 끈 상태면 잠깐 돌고) 스피너를 거둔다
  useEffect(() => {
    if (!refreshing) return;
    if (busy) sawBusy.current = true;
    const finish = () => {
      refreshingRef.current = false;
      setRefreshing(false);
      pullRef.current = 0;
      setPull(0);
    };
    const wait = Math.max(0, MIN_SPIN_MS - (Date.now() - startedAt.current));
    if (!busy) {
      const t = window.setTimeout(finish, wait);
      return () => window.clearTimeout(t);
    }
    const cap = window.setTimeout(finish, MAX_SPIN_MS);
    return () => window.clearTimeout(cap);
  }, [refreshing, busy]);

  const progress = Math.min(1, pull / THRESHOLD);
  const shown = refreshing ? SPOKES : Math.round(progress * SPOKES);
  return (
    <div className="ptr">
      <div className="ptr-indicator" style={{ height: pull, opacity: pull > 4 ? 1 : 0 }} aria-hidden={!refreshing}>
        <span className={`ptr-spinner ${refreshing ? "spinning" : ""}`} role={refreshing ? "status" : undefined} aria-label={refreshing ? "새로고침 중" : undefined}>
          {Array.from({ length: SPOKES }, (_, i) => (
            <i
              key={i}
              style={{
                transform: `rotate(${i * (360 / SPOKES)}deg)`,
                opacity: i < shown ? (refreshing ? 0.25 + (0.75 * (i + 1)) / SPOKES : 0.85) : 0,
              }}
            />
          ))}
        </span>
      </div>
      <div className="ptr-content" style={{ transform: pull ? `translateY(${pull}px)` : undefined, transition: dragging ? "none" : undefined }}>
        {children}
      </div>
    </div>
  );
}

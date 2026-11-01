import { useSyncExternalStore } from "react";
import { api, Recording } from "./api";

// 예전 녹음에 AI 제목, 태그, 할 일을 하나씩 붙이는 일괄 작업. 설정 창을 닫아도 이어지도록 화면 컴포넌트 밖에 둔다.
// 진행 상황은 설정 줄과 사이드바 아래에 함께 보인다. 앱을 끄면 지금 녹음에서 멈추고, 끝난 녹음은 그대로 남는다.
// @see docs/organize.md

export interface BulkState {
  running: boolean;
  /** 끝낸 개수 */
  done: number;
  total: number;
  failed: number;
  /** 지금 정리하는 녹음 */
  current: Recording | null;
  /** 지금 녹음을 시작한 시각(ms). 경과 시간 표시용 */
  currentStartedAt: number | null;
  /** 멈추기를 눌렀다. 지금 녹음이 끝나면 멈춘다 */
  stopping: boolean;
  /** 끝났을 때 문구 */
  message: string | null;
}

let state: BulkState = { running: false, done: 0, total: 0, failed: 0, current: null, currentStartedAt: null, stopping: false, message: null };
const listeners = new Set<() => void>();

function set(patch: Partial<BulkState>) {
  state = { ...state, ...patch };
  listeners.forEach((l) => l());
}

export function useBulkOrganize(): BulkState {
  return useSyncExternalStore(
    (l) => {
      listeners.add(l);
      return () => listeners.delete(l);
    },
    () => state,
  );
}

/** force: 이미 정리한 녹음도 지금 규칙으로 다시 정리한다 */
export async function startBulkOrganize(targets: Recording[], force: boolean, onEach?: () => void) {
  if (state.running || targets.length === 0) return;
  set({ running: true, done: 0, total: targets.length, failed: 0, current: null, currentStartedAt: null, stopping: false, message: null });
  let done = 0;
  let failed = 0;
  for (const r of targets) {
    if (state.stopping) break;
    set({ current: r, currentStartedAt: Date.now() });
    try {
      await api.organize(r.id, force);
    } catch (e) {
      failed += 1;
      console.warn(`정리 실패 ${r.title}: ${e}`);
    }
    done += 1;
    set({ done, failed });
    onEach?.();
  }
  const stopped = done < targets.length;
  const ok = done - failed;
  const message = stopped
    ? `${targets.length}개 중 ${done}개에서 멈췄습니다(${ok}개 붙임${failed ? `, ${failed}개 실패` : ""})`
    : failed
      ? `${ok}개에 붙였고 ${failed}개는 실패했습니다`
      : `${ok}개에 제목, 태그, 할 일을 붙였습니다`;
  set({ running: false, current: null, currentStartedAt: null, stopping: false, message });
}

/** 지금 녹음이 끝나면 멈춘다. AI 호출 하나는 중간에 끊지 않는다(끊어도 다시 하면 된다) */
export function stopBulkOrganize() {
  if (state.running) set({ stopping: true });
}

export function clearBulkMessage() {
  set({ message: null });
}

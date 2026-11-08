import { useLayoutEffect, useState } from "react";
import { api, GlobalShortcuts } from "../api";
import { acceleratorLabel, SHORTCUTS, shortcutLabel } from "../shortcuts";

// ⌘(윈도우는 Ctrl)를 누르고 있는 동안 버튼 위에 단축키를 띄운다.
// 버튼은 `data-shortcut="<명령>"`(앱 안 단축키) 또는 `data-shortcut-global="<설정 키>"`(전역 단축키)로 표시한다.
// @see docs/shortcuts-and-deep-links.md

interface Badge {
  id: number;
  x: number;
  y: number;
  label: string;
}

/** 배지 높이의 절반 정도. 창 위쪽 버튼의 배지가 잘리지 않게 이만큼 내린다 */
const EDGE = 12;

/** 다른 요소에 가려지거나 스크롤 영역 밖으로 잘린 버튼은 건너뛴다. 오버레이는 pointer-events가 없어 판정에 끼지 않는다 */
function visibleRect(el: HTMLElement): DOMRect | null {
  const rect = el.getBoundingClientRect();
  if (rect.width === 0 || rect.height === 0) return null;
  const cx = rect.left + rect.width / 2;
  const cy = rect.top + rect.height / 2;
  if (cx < 0 || cy < 0 || cx > window.innerWidth || cy > window.innerHeight) return null;
  const hit = document.elementFromPoint(cx, cy);
  return hit && el.contains(hit) ? rect : null;
}

function measure(global: GlobalShortcuts | null): { badges: Badge[]; shown: Set<string> } {
  const badges: Badge[] = [];
  const shown = new Set<string>();
  document.querySelectorAll<HTMLElement>("[data-shortcut], [data-shortcut-global]").forEach((el) => {
    const run = el.dataset.shortcut;
    const globalKey = el.dataset.shortcutGlobal as keyof GlobalShortcuts | undefined;
    let label: string | null = null;
    if (run) {
      const shortcut = SHORTCUTS.find((s) => s.run === run);
      if (shortcut) label = shortcutLabel(shortcut);
    } else if (globalKey && global?.[globalKey]) {
      label = acceleratorLabel(global[globalKey]);
    }
    if (!label) return;
    const rect = visibleRect(el);
    if (!rect) return;
    if (run) shown.add(run);
    badges.push({
      id: badges.length,
      x: rect.left + rect.width / 2,
      y: Math.max(rect.top, EDGE),
      label,
    });
  });
  return { badges, shown };
}

export function ShortcutOverlay() {
  const [global, setGlobal] = useState<GlobalShortcuts | null>(null);
  const [state, setState] = useState<{ badges: Badge[]; shown: Set<string> }>({ badges: [], shown: new Set() });

  useLayoutEffect(() => {
    api.getSettings().then((s) => setGlobal(s.shortcuts)).catch(() => {});
  }, []);

  useLayoutEffect(() => {
    const update = () => setState(measure(global));
    update();
    window.addEventListener("resize", update);
    return () => window.removeEventListener("resize", update);
  }, [global]);

  // 화면에 버튼이 없는 ⌘ 단축키는 아래 줄에 모은다. 같은 명령에 키가 둘이면(⌘K, ⌘F) 버튼 배지 하나로 충분하다
  const rest = SHORTCUTS.filter((s) => s.mod && !state.shown.has(s.run));

  return (
    <div className="shortcut-overlay" aria-hidden>
      {state.badges.map((b) => (
        <kbd key={b.id} className="shortcut-badge" style={{ left: b.x, top: b.y }}>
          {b.label}
        </kbd>
      ))}
      {rest.length > 0 && (
        <div className="shortcut-rest">
          {rest.map((s) => (
            <span key={s.label}>
              <kbd>{shortcutLabel(s)}</kbd>
              {s.label}
            </span>
          ))}
        </div>
      )}
    </div>
  );
}

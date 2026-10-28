import { useEffect, useState } from "react";
import { api, GlobalShortcuts } from "../api";
import { acceleratorLabel, Shortcut, SHORTCUTS, shortcutLabel } from "../shortcuts";

const GROUPS: Shortcut["group"][] = ["파일", "처리", "이동", "재생", "보기"];

export function ShortcutHelp({ onClose }: { onClose: () => void }) {
  const [global, setGlobal] = useState<GlobalShortcuts | null>(null);

  useEffect(() => {
    api.getSettings().then((s) => setGlobal(s.shortcuts));
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div className="modal shortcut-help" role="dialog" aria-label="단축키" onClick={(e) => e.stopPropagation()}>
        <h2>단축키</h2>
        {global && (
          <section>
            <h3>sori가 앞에 없어도</h3>
            <dl className="shortcut-table">
              <div className="shortcut-row">
                <dt>녹음 시작/멈춤</dt>
                <dd><kbd>{acceleratorLabel(global.toggleRecording)}</kbd></dd>
              </div>
              <div className="shortcut-row">
                <dt>sori 창 앞으로</dt>
                <dd><kbd>{acceleratorLabel(global.showWindow)}</kbd></dd>
              </div>
            </dl>
          </section>
        )}
        {GROUPS.map((g) => (
          <section key={g}>
            <h3>{g}</h3>
            <dl className="shortcut-table">
              {SHORTCUTS.filter((s) => s.group === g).map((s) => (
                <div key={s.label} className="shortcut-row">
                  <dt>{s.label}</dt>
                  <dd><kbd>{shortcutLabel(s)}</kbd></dd>
                </div>
              ))}
            </dl>
          </section>
        ))}
        <p className="muted hint">전역 단축키는 설정에서 바꿀 수 있습니다. 재생 단축키는 글자 입력칸에 커서가 있으면 동작하지 않습니다.</p>
        <div className="modal-actions">
          <button onClick={onClose}>닫기</button>
        </div>
      </div>
    </div>
  );
}

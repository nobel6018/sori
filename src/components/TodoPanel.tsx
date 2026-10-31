import { useCallback, useEffect, useState } from "react";
import { api, Todo } from "../api";

interface Props {
  recordingId: string;
  /** AI 정리가 끝날 때마다 바뀐다. 다시 읽는 신호 */
  version: string | undefined;
}

/** AI가 뽑은 할 일 후보. 사용자가 고르고 고친 것만 "미리 알림에 넣기"를 눌러야 들어간다
 * @see docs/organize.md */
export function TodoPanel({ recordingId, version }: Props) {
  const [todos, setTodos] = useState<Todo[]>([]);
  const [picked, setPicked] = useState<Set<string>>(new Set());
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<string | null>(null);

  const load = useCallback(() => {
    api
      .getTodos(recordingId)
      .then((list) => {
        setTodos(list);
        // 새 후보는 기본으로 고른다
        setPicked(new Set(list.filter((t) => !t.addedAt).map((t) => t.id)));
      })
      .catch(() => setTodos([]));
  }, [recordingId]);

  useEffect(load, [load, version]);

  if (todos.length === 0) return null;
  const pending = todos.filter((t) => !t.addedAt);

  const edit = (id: string, change: Partial<Todo>) => setTodos((prev) => prev.map((t) => (t.id === id ? { ...t, ...change } : t)));
  const save = (next: Todo[]) => api.saveTodos(recordingId, next).catch((e) => setMessage(String(e)));
  const remove = (id: string) => {
    const next = todos.filter((t) => t.id !== id);
    setTodos(next);
    save(next);
  };
  const toggle = (id: string) =>
    setPicked((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });

  const add = async () => {
    const ids = pending.filter((t) => picked.has(t.id) && t.text.trim()).map((t) => t.id);
    if (!ids.length) return;
    setBusy(true);
    setMessage(null);
    try {
      // 고친 내용을 먼저 저장해야 고친 글로 들어간다
      await api.saveTodos(recordingId, todos);
      const added = await api.addTodosToReminders(recordingId, ids);
      setMessage(`미리 알림에 ${added}개를 넣었습니다`);
      load();
    } catch (e) {
      setMessage(String(e));
    } finally {
      setBusy(false);
    }
  };

  const count = pending.filter((t) => picked.has(t.id)).length;

  return (
    <section className="todo-panel">
      <h3>할 일 후보</h3>
      <ul>
        {todos.map((t) => (
          <li key={t.id} className={t.addedAt ? "added" : ""}>
            {t.addedAt ? (
              <>
                <span className="todo-text">{t.text}</span>
                {t.due && <span className="muted">{t.due}</span>}
                <span className="muted">넣음</span>
              </>
            ) : (
              <>
                <input type="checkbox" checked={picked.has(t.id)} onChange={() => toggle(t.id)} aria-label={`${t.text} 고르기`} />
                <input
                  className="todo-text"
                  value={t.text}
                  aria-label="할 일"
                  onChange={(e) => edit(t.id, { text: e.target.value })}
                  onBlur={() => save(todos)}
                />
                <input
                  type="date"
                  value={t.due ?? ""}
                  aria-label="기한"
                  onChange={(e) => edit(t.id, { due: e.target.value || null })}
                  onBlur={() => save(todos)}
                />
                <button className="ghost" onClick={() => remove(t.id)} aria-label={`${t.text} 빼기`} title="후보에서 뺍니다">
                  ×
                </button>
              </>
            )}
          </li>
        ))}
      </ul>
      {pending.length > 0 && (
        <div className="todo-actions">
          <button className="primary" disabled={busy || count === 0} onClick={add}>
            {busy ? "넣는 중" : `미리 알림에 넣기 (${count}개)`}
          </button>
          <span className="muted">고른 할 일만 미리 알림의 기본 목록에 넣습니다. 처음에 macOS가 허락을 묻습니다</span>
        </div>
      )}
      {message && <p className="muted">{message}</p>}
    </section>
  );
}

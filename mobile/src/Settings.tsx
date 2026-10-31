import { useState } from "react";
import { api, SyncStatus, syncLabel } from "./api";

export function Settings({ sync, onBack }: { sync: SyncStatus | null; onBack: () => void }) {
  const [error, setError] = useState<string | null>(null);
  const provider = sync?.provider ?? "local";

  const chooseIcloud = async () => {
    setError(null);
    try {
      await api.chooseIcloudFolder();
    } catch (e) {
      setError(String(e));
    }
  };

  const useLocal = async () => {
    if (!confirm("동기화를 끌까요? 이미 받은 녹음은 이 기기에 남습니다.")) return;
    await api.useLocalOnly().catch((e) => setError(String(e)));
  };

  return (
    <div className="screen">
      <header className="bar">
        <button className="text-button" onClick={onBack}>
          ‹ 목록
        </button>
        <h1 className="bar-title">설정</h1>
        <span />
      </header>
      <main className="content settings">
        <section className="card">
          <h2>동기화</h2>
          <div className="choice" role="radiogroup" aria-label="저장 위치">
            <button role="radio" aria-checked={provider === "local"} className={provider === "local" ? "on" : ""} onClick={useLocal} disabled={provider === "local"}>
              <strong>이 기기에만</strong>
              <span className="muted small">동기화하지 않습니다</span>
            </button>
            <button role="radio" aria-checked={provider === "icloud"} className={provider === "icloud" ? "on" : ""} onClick={chooseIcloud}>
              <strong>iCloud Drive</strong>
              <span className="muted small">
                {provider === "icloud" && sync?.folder ? `폴더: ${sync.folder} (눌러서 바꾸기)` : "Mac의 sori와 녹음, 전사, 요약, 메모를 나눕니다"}
              </span>
            </button>
          </div>
          <p className="muted small">
            Mac의 sori 설정에서 동기화를 iCloud Drive로 켜면 iCloud Drive에 <b>sori</b> 폴더가 생깁니다. 여기서 iCloud Drive를 누르고 그
            폴더를 고르세요. 안의 recordings 폴더나 iCloud Drive 자체를 골라도 sori 폴더로 맞춰 씁니다.
          </p>
          {provider === "icloud" && sync && (
            <div className="sync-line">
              <span className={sync.error ? "error" : "muted"}>{sync.error ?? syncLabel(sync)}</span>
              <button onClick={() => api.syncNow()} disabled={sync.running}>
                지금 동기화
              </button>
            </div>
          )}
          {error && <p className="error">{error}</p>}
        </section>

        <section className="card">
          <h2>녹음</h2>
          <p className="muted small">
            AAC 모노 64kbps로 녹음합니다(1시간 약 28MB). 받아 적기와 요약은 Mac의 sori가 하고, 끝나면 이 앱에 나타납니다.
          </p>
        </section>
      </main>
    </div>
  );
}

import { useEffect, useRef, useState } from "react";
import { api, DownloadProgress, ModelCatalog } from "./api";

export interface InstallState {
  running: boolean;
  progress: DownloadProgress | null;
  /** 최근 몇 초 평균 속도(바이트/초) */
  speed: number;
  error: string | null;
  start: (whisper: string[]) => Promise<ModelCatalog | null>;
  cancel: () => void;
}

/** 모델 내려받기 상태. 첫 실행 안내와 설정 화면이 같이 쓴다 */
export function useModelInstall(): InstallState {
  const [running, setRunning] = useState(false);
  const [progress, setProgress] = useState<DownloadProgress | null>(null);
  const [speed, setSpeed] = useState(0);
  const [error, setError] = useState<string | null>(null);
  // 속도는 순간값이 출렁여서 최근 5초 표본으로 평균 낸다
  const samples = useRef<{ t: number; bytes: number }[]>([]);

  useEffect(() => {
    const unlisten = api.onModelDownload((p) => {
      setProgress(p);
      const now = performance.now();
      const s = samples.current;
      s.push({ t: now, bytes: p.received });
      while (s.length > 2 && now - s[0].t > 5000) s.shift();
      // 표본이 1.5초 이상 쌓이기 전에는 속도를 내지 않는다. 첫 표본만으로 재면 "116분 남음" 같은 값이 나온다
      if (s.length >= 2 && s[s.length - 1].t - s[0].t >= 1500) {
        setSpeed(((s[s.length - 1].bytes - s[0].bytes) / (s[s.length - 1].t - s[0].t)) * 1000);
      }
    });
    return () => {
      unlisten.then((f) => f());
    };
  }, []);

  const start = async (whisper: string[]) => {
    setRunning(true);
    setError(null);
    setProgress(null);
    setSpeed(0);
    samples.current = [];
    try {
      return await api.installModels(whisper);
    } catch (e) {
      setError(String(e));
      return null;
    } finally {
      setRunning(false);
    }
  };

  return { running, progress, speed, error, start, cancel: () => api.cancelModelInstall() };
}

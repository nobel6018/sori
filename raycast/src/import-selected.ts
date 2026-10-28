import { getSelectedFinderItems, showHUD } from "@raycast/api";
import { openSori } from "./store";

// sori 앱이 가져올 수 있는 확장자. src/api.ts AUDIO_EXTENSIONS와 같다
const AUDIO = ["m4a", "mp3", "wav", "aac", "flac", "ogg", "mp4", "mov", "mkv", "webm", "caf"];

export default async function Command() {
  let paths: string[];
  try {
    paths = (await getSelectedFinderItems()).map((i) => i.path);
  } catch {
    await showHUD("Finder에서 오디오 파일을 먼저 고르세요");
    return;
  }
  const audio = paths.filter((p) => AUDIO.includes(p.split(".").pop()?.toLowerCase() ?? ""));
  if (audio.length === 0) {
    await showHUD("고른 파일 중 오디오가 없습니다");
    return;
  }
  try {
    await openSori("import", [...audio.map((p): [string, string] => ["path", p]), ["process", "1"]]);
    await showHUD(`${audio.length}개를 sori로 보냈습니다. 바로 받아 적습니다`);
  } catch (e) {
    await showHUD(String(e instanceof Error ? e.message : e));
  }
}

import { showHUD } from "@raycast/api";
import { listRecordings, openSori } from "./store";

export default async function Command() {
  const recording = (await listRecordings()).some((r) => r.status === "recording");
  try {
    await openSori("record/toggle");
    await showHUD(recording ? "녹음을 멈춥니다" : "녹음을 시작합니다");
  } catch (e) {
    await showHUD(String(e instanceof Error ? e.message : e));
  }
}

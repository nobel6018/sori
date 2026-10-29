import { showHUD } from "@raycast/api";
import { listRecordings, openSori } from "./store";

export default async function Command() {
  if (!(await listRecordings()).some((r) => r.status === "recording")) {
    await showHUD("녹음 중이 아닙니다");
    return;
  }
  try {
    await openSori("record/bookmark");
    await showHUD("북마크를 남겼습니다");
  } catch (e) {
    await showHUD(String(e instanceof Error ? e.message : e));
  }
}

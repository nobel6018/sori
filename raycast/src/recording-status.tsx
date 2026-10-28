import { Icon, launchCommand, LaunchType, MenuBarExtra, showHUD } from "@raycast/api";
import { useEffect, useState } from "react";
import { formatTime, listRecordings, openSori as open, Recording } from "./store";

const openSori = (path: string) => open(path).catch((e) => showHUD(e instanceof Error ? e.message : String(e)));

// 10초마다 다시 그린다(package.json interval). 녹음 중이면 경과 시간을 보여 준다
export default function Command() {
  const [active, setActive] = useState<Recording | null | undefined>(undefined);

  useEffect(() => {
    listRecordings().then((all) => setActive(all.find((r) => r.status === "recording") ?? null));
  }, []);

  const elapsed = active ? formatTime((Date.now() - new Date(active.createdAt).getTime()) / 1000) : "";
  return (
    <MenuBarExtra
      isLoading={active === undefined}
      icon={active ? { source: Icon.CircleFilled, tintColor: "#d23c2f" } : Icon.Microphone}
      title={active ? elapsed : undefined}
      tooltip={active ? `녹음 중: ${active.title}` : "sori"}
    >
      {active ? (
        <MenuBarExtra.Item title="녹음 멈추기" icon={Icon.Stop} onAction={() => openSori("record/stop")} />
      ) : (
        <MenuBarExtra.Item title="녹음 시작" icon={Icon.Microphone} onAction={() => openSori("record/start")} />
      )}
      <MenuBarExtra.Section>
        <MenuBarExtra.Item title="전사 검색" icon={Icon.MagnifyingGlass} onAction={() => launchCommand({ name: "search-transcripts", type: LaunchType.UserInitiated })} />
        <MenuBarExtra.Item title="sori 열기" icon={Icon.AppWindow} onAction={() => openSori("show")} />
      </MenuBarExtra.Section>
    </MenuBarExtra>
  );
}

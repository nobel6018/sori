import { Menu } from "@tauri-apps/api/menu";
import { api, Recording } from "./api";
import { EXPORT_LABEL, ExportFormat, exportBytes, exportFileName } from "./export";
import { recordingMarkdown } from "./markdown";
import { IS_MAC } from "./shortcuts";

// 사이드바 녹음 목록의 우클릭 메뉴. 웹뷰 기본 메뉴(다시 불러오기) 대신 OS 메뉴를 띄운다
// @see docs/context-menu.md

export interface RecordingMenuContext {
  /** 기본 설정으로 처리한다. 자동 요약, AI 정리 설정도 따른다 */
  process: (recording: Recording) => Promise<void>;
  refresh: () => Promise<void>;
  onDeleted: (id: string) => Promise<void>;
  onError: (message: string) => void;
}

/** 녹음 상태에 맞는 동작만 켜서 커서 위치에 메뉴를 띄운다. 처리 중인 녹음은 지우거나 다시 처리하지 않는다 */
export async function showRecordingMenu(recording: Recording, ctx: RecordingMenuContext): Promise<void> {
  const { id, status, title } = recording;
  const busy = status === "recording" || status === "processing";
  const done = status === "done";
  const fail = (e: unknown) => ctx.onError(`${title}: ${e}`);

  const exportAs = async (format: ExportFormat) => {
    try {
      await api.saveExport(exportFileName(title, format), await exportBytes(await api.getRecording(id), format));
    } catch (e) {
      fail(e);
    }
  };

  const menu = await Menu.new({
    items: [
      {
        text: done ? "기본 설정으로 다시 받아 적기" : "받아 적기",
        enabled: !busy,
        action: () => ctx.process(recording).catch(fail),
      },
      ...(status === "processing" ? [{ text: "처리 취소", action: () => api.cancelProcessing(id).catch(fail) }] : []),
      {
        text: "요약하기",
        enabled: done,
        action: () => api.summarize(id, null, null).catch(fail),
      },
      {
        text: "AI로 제목, 태그 다시 정리",
        enabled: done,
        action: () => api.organize(id, true).then(ctx.refresh).catch(fail),
      },
      { item: "Separator" },
      {
        text: "마크다운으로 복사",
        enabled: done,
        action: async () => {
          try {
            await navigator.clipboard.writeText(recordingMarkdown(await api.getRecording(id)));
          } catch (e) {
            fail(e);
          }
        },
      },
      {
        text: "내보내기",
        enabled: done,
        items: (Object.keys(EXPORT_LABEL) as ExportFormat[]).map((format) => ({
          text: EXPORT_LABEL[format],
          action: () => exportAs(format),
        })),
      },
      {
        text: IS_MAC ? "Finder에서 보기" : "탐색기에서 보기",
        enabled: status !== "recording",
        action: () => api.revealAudio(id).catch(fail),
      },
      { item: "Separator" },
      {
        text: "삭제…",
        enabled: !busy,
        action: async () => {
          if (!confirm(`"${title}" 녹음과 전사 결과를 삭제할까요? 되돌릴 수 없습니다.`)) return;
          try {
            await api.deleteRecording(id);
            await ctx.onDeleted(id);
          } catch (e) {
            fail(e);
          }
        },
      },
    ],
  });
  await menu.popup();
}

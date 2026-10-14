import { useCallback, useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { toast } from "sonner";
import { commands, type ModelFileList } from "@/bindings";

// Mirrors the progress event payload in commands/models.rs.
interface DownloadProgress {
  id: string;
  downloaded: number;
  total: number;
}

/** The offline voice's files, kept current from the backend's events. */
export const useModelFiles = () => {
  const [list, setList] = useState<ModelFileList | null>(null);
  const [progress, setProgress] = useState<Record<string, number>>({});

  const refresh = useCallback(async () => {
    const result = await commands.listModelFiles();
    if (result.status !== "ok") return;
    setList(result.data);
    // Live counts only matter while downloading; a restart reports afresh.
    const running = new Set(
      result.data.files.filter((f) => f.downloading).map((f) => f.id),
    );
    setProgress((current) =>
      Object.fromEntries(
        Object.entries(current).filter(([id]) => running.has(id)),
      ),
    );
  }, []);

  useEffect(() => {
    refresh();
    const unlisteners = [
      listen("model-files-changed", () => refresh()),
      listen<DownloadProgress>("model-download-progress", ({ payload }) =>
        setProgress((current) => ({
          ...current,
          [payload.id]: payload.downloaded,
        })),
      ),
    ];
    return () => {
      unlisteners.forEach((unlisten) => unlisten.then((stop) => stop()));
    };
  }, [refresh]);

  /** Bytes so far: live while downloading, else what a resume starts from. */
  const downloaded = (id: string) => {
    const file = list?.files.find((f) => f.id === id);
    if (!file) return 0;
    return file.downloading ? (progress[id] ?? file.partial) : file.partial;
  };

  const run = async (
    action: (id: string) => Promise<{ status: string; error?: string }>,
    id: string,
  ) => {
    const result = await action(id);
    if (result.status === "error" && result.error) toast.error(result.error);
  };

  return {
    list,
    downloaded,
    download: (id: string) => run(commands.downloadModelFile, id),
    cancel: (id: string) => run(commands.cancelModelDownload, id),
    remove: (id: string) => run(commands.deleteModelFile, id),
    openFolder: () => commands.openModelsDir(),
  };
};

import { listen } from "@tauri-apps/api/event";
import React, { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import "./SpeakingOverlay.css";
import { commands } from "@/bindings";
import { syncLanguageFromSettings } from "@/i18n";

// Mirrors `OverlayView` in overlay.rs.
type OverlayView =
  | { kind: "speaking" }
  | { kind: "nothing-selected" }
  | { kind: "error"; message: string };

const SpeakingOverlay: React.FC = () => {
  const { t } = useTranslation();
  const [view, setView] = useState<OverlayView | null>(null);
  const [visible, setVisible] = useState(false);
  const [top, setTop] = useState(false);

  useEffect(() => {
    // A show resolves asynchronously; drop it if a later show or hide overtook it.
    let latest = 0;
    const show = listen<OverlayView>("show-overlay", async ({ payload }) => {
      const seq = ++latest;
      await syncLanguageFromSettings();
      const settings = await commands.getAppSettings().catch(() => null);
      if (seq !== latest) return;
      if (settings?.status === "ok") {
        setTop(settings.data.overlay_position === "top");
      }
      setView(payload);
      setVisible(true);
    });
    const hide = listen("hide-overlay", () => {
      latest++;
      setVisible(false);
    });
    return () => {
      show.then((unlisten) => unlisten());
      hide.then((unlisten) => unlisten());
    };
  }, []);

  if (!view) return null;

  const message =
    view.kind === "error"
      ? `${t("overlay.error")}: ${view.message}`
      : t("overlay.nothingSelected");

  return (
    <div className={`ov-stage${top ? " top" : ""}${visible ? " visible" : ""}`}>
      <div className={`ov-card ${view.kind}`} role="status">
        {view.kind === "speaking" ? (
          <>
            <span className="ov-bars" aria-hidden="true">
              <i />
              <i />
              <i />
              <i />
            </span>
            <span className="ov-text">{t("overlay.speaking")}</span>
          </>
        ) : (
          <span className="ov-text" title={message}>
            {message}
          </span>
        )}
        {view.kind === "error" && (
          <button
            className="ov-button"
            onClick={() => commands.showMainWindowCommand()}
          >
            {t("overlay.openSettings")}
          </button>
        )}
        <button
          className="ov-close"
          aria-label={t(
            view.kind === "speaking" ? "tray.stop" : "overlay.dismiss",
          )}
          onClick={() => commands.stopSpeaking()}
        >
          <svg viewBox="0 0 16 16" aria-hidden="true">
            <path
              d="M4 4 L12 12 M12 4 L4 12"
              stroke="currentColor"
              strokeWidth="1.6"
              strokeLinecap="round"
            />
          </svg>
        </button>
      </div>
    </div>
  );
};

export default SpeakingOverlay;

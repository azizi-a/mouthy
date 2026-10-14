import { emitTo, listen } from "@tauri-apps/api/event";
import { X } from "lucide-react";
import React, { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import "./SpeakingOverlay.css";
import { commands } from "@/bindings";
import { syncLanguageFromSettings } from "@/i18n";

// Mirrors `OverlayView` in overlay.rs.
type OverlayView =
  | { kind: "speaking" }
  | { kind: "nothing-selected" }
  | { kind: "error"; message: string }
  | { kind: "model-missing" };

const SpeakingOverlay: React.FC = () => {
  const { t } = useTranslation();
  const [view, setView] = useState<OverlayView | null>(null);
  const [visible, setVisible] = useState(false);

  useEffect(() => {
    // A show resolves asynchronously; drop it if a later show or hide overtook it.
    let latest = 0;
    const show = listen<OverlayView>("show-overlay", async ({ payload }) => {
      const seq = ++latest;
      await syncLanguageFromSettings();
      if (seq !== latest) return;
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
      : view.kind === "model-missing"
        ? t("overlay.modelMissing")
        : t("overlay.nothingSelected");
  const opensSettings = view.kind === "error" || view.kind === "model-missing";
  const openSettings = async () => {
    if (view.kind === "model-missing") {
      await emitTo("main", "show-section", "models");
    }
    await commands.showMainWindowCommand();
  };

  return (
    <div className={`ov-stage${visible ? " visible" : ""}`}>
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
          <span className="ov-text">{message}</span>
        )}
        {opensSettings && (
          <button className="ov-button" onClick={openSettings}>
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
          <X size={12} strokeWidth={2.5} />
        </button>
      </div>
    </div>
  );
};

export default SpeakingOverlay;

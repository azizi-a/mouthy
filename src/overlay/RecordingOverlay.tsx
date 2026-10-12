import { listen } from "@tauri-apps/api/event";
import React, { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import "./RecordingOverlay.css";
import { commands } from "@/bindings";
import i18n, { syncLanguageFromSettings } from "@/i18n";
import { getLanguageDirection } from "@/lib/utils/rtl";

// Mirrors `OverlayView` in overlay.rs.
type OverlayView =
  | { kind: "speaking" }
  | { kind: "nothing-selected" }
  | { kind: "error"; message: string };

// Number of bars in the activity pulse.
const WAVE_BARS = 9;

const RecordingOverlay: React.FC = () => {
  const { t } = useTranslation();
  const [isVisible, setIsVisible] = useState(false);
  const [view, setView] = useState<OverlayView>({ kind: "speaking" });
  // Overlay placement (top vs bottom of the screen); the card sits flush to
  // that edge.
  const [position, setPosition] = useState<"top" | "bottom">("bottom");
  const direction = getLanguageDirection(i18n.language);

  useEffect(() => {
    const present = async (next: OverlayView) => {
      await syncLanguageFromSettings();
      try {
        const settings = await commands.getAppSettings();
        if (settings.status === "ok") {
          setPosition(
            settings.data.overlay_position === "top" ? "top" : "bottom",
          );
        }
      } catch {
        // Keep the previous/default placement if settings can't be read.
      }
      setView(next);
      setIsVisible(true);
    };

    const setupEventListeners = async () => {
      const unlistenShow = await listen<OverlayView>("show-overlay", (event) =>
        present(event.payload),
      );

      const unlistenHide = await listen("hide-overlay", () => {
        setIsVisible(false);
      });

      return () => {
        unlistenShow();
        unlistenHide();
      };
    };

    setupEventListeners();
  }, []);

  if (!isVisible) return null;

  const stopBtn = (
    <button
      className="sx"
      aria-label={t("tray.stop")}
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
  );

  // dot (left) | pulse (center) | stop (right)
  const speakingRow = (
    <div className="sbase" role="status" aria-label={t("overlay.speaking")}>
      <div className="sbase-l">
        <span className="sdot" />
      </div>
      <div className="swave">
        {Array.from({ length: WAVE_BARS }, (_, i) => (
          <i key={i} />
        ))}
      </div>
      <div className="sbase-r">{stopBtn}</div>
    </div>
  );

  // (empty) | label (center) | dismiss (right)
  const noticeRow = (label: string) => (
    <div className="sbase" role="status">
      <div className="sbase-l" />
      <span className="swork-label" title={label}>
        {label}
      </span>
      <div className="sbase-r">{stopBtn}</div>
    </div>
  );

  let row = speakingRow;
  if (view.kind === "nothing-selected") {
    row = noticeRow(t("overlay.nothingSelected"));
  } else if (view.kind === "error") {
    row = noticeRow(
      view.message
        ? `${t("overlay.error")}: ${view.message}`
        : t("overlay.error"),
    );
  }

  return (
    <div
      dir={direction}
      className={`ov-stage ${position} ov-fade ${isVisible ? "show" : ""}`}
    >
      <div
        className={`scard compact ${view.kind !== "speaking" ? "cworking" : ""}`}
      >
        {row}
      </div>
    </div>
  );
};

export default RecordingOverlay;

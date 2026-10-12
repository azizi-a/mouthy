import { listen } from "@tauri-apps/api/event";
import React, { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import "./RecordingOverlay.css";
import { commands } from "@/bindings";
import i18n, { syncLanguageFromSettings } from "@/i18n";
import type { ModelStateEvent } from "@/lib/types/events";
import { getLanguageDirection } from "@/lib/utils/rtl";

type OverlayState = "recording" | "transcribing" | "processing";

// Number of bars in the activity pulse.
const WAVE_BARS = 9;

const RecordingOverlay: React.FC = () => {
  const { t } = useTranslation();
  const [isVisible, setIsVisible] = useState(false);
  const [state, setState] = useState<OverlayState>("recording");
  const [modelLoading, setModelLoading] = useState(false);
  // Overlay placement (top vs bottom of the screen); the card sits flush to
  // that edge.
  const [position, setPosition] = useState<"top" | "bottom">("bottom");
  const direction = getLanguageDirection(i18n.language);

  useEffect(() => {
    const setupEventListeners = async () => {
      const unlistenShow = await listen("show-overlay", async (event) => {
        const overlayState = event.payload as OverlayState;
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
        setState(overlayState);
        setIsVisible(true);
      });

      const unlistenHide = await listen("hide-overlay", () => {
        setIsVisible(false);
      });

      // The backend ends every `loading_started` with exactly one of completed
      // or failed, so only those end a load. Other events (e.g. `unloaded` from
      // the idle watcher) aren't ordered against an in-flight load and must not
      // clear it.
      const unlistenModel = await listen<ModelStateEvent>(
        "model-state-changed",
        (event) => {
          const type = event.payload.event_type;
          if (type === "loading_started") {
            setModelLoading(true);
          } else if (
            type === "loading_completed" ||
            type === "loading_failed"
          ) {
            setModelLoading(false);
          }
        },
      );

      return () => {
        unlistenShow();
        unlistenHide();
        unlistenModel();
      };
    };

    setupEventListeners();
  }, []);

  if (!isVisible) return null;

  const transcribingLabel = modelLoading
    ? t("overlay.loadingModel")
    : t("overlay.transcribing");

  const cancelBtn = (
    <button
      className="sx"
      aria-label="cancel"
      onClick={() => commands.cancelOperation()}
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

  // dot (left) | pulse (center) | cancel (right)
  const listeningRow = (
    <div className="sbase">
      <div className="sbase-l">
        <span className="sdot" />
      </div>
      <div className="swave">
        {Array.from({ length: WAVE_BARS }, (_, i) => (
          <i key={i} />
        ))}
      </div>
      <div className="sbase-r">{cancelBtn}</div>
    </div>
  );

  // spinner (left) | label (center) | cancel (right) — same 3-zone grid as the
  // listening row, so the label is centered.
  const workingRow = (label: string) => (
    <div className="sbase">
      <div className="sbase-l">
        <span className="sspinner" />
      </div>
      <span className="swork-label">{label}</span>
      <div className="sbase-r">{cancelBtn}</div>
    </div>
  );

  // Exactly one row at a time: the pulse (recording), or a spinner + label
  // (transcribing / processing). The pill animates its width between them; the
  // cancel button is in both rows so it stays put.
  const working = state === "transcribing" || state === "processing";
  const workLabel =
    state === "processing" ? t("overlay.processing") : transcribingLabel;

  return (
    <div
      dir={direction}
      className={`ov-stage ${position} ov-fade ${isVisible ? "show" : ""}`}
    >
      <div
        className={`scard compact ${working && isVisible ? "cworking" : ""}`}
      >
        {working ? workingRow(workLabel) : listeningRow}
      </div>
    </div>
  );
};

export default RecordingOverlay;

import React from "react";
import { useTranslation } from "react-i18next";
import type { ModelFile, VoiceLanguage } from "@/bindings";
import { useSettings } from "../../hooks/useSettings";
import { useVoicePreview } from "../../hooks/useVoicePreview";
import { Button } from "../ui/Button";
import { FileRow, useMegabytes } from "./FileRow";

// Mirrors `VoiceLanguage::of` in catalog.rs.
const LANGUAGES: Record<string, VoiceLanguage> = {
  a: "americanEnglish",
  b: "britishEnglish",
  j: "japanese",
  z: "mandarin",
  e: "spanish",
  f: "french",
  h: "hindi",
  i: "italian",
  p: "brazilianPortuguese",
};

/** The language a Kokoro-style id (`bf_emma`) names, if it is one. */
export const languageOf = (id: string): VoiceLanguage | null =>
  /^[a-z][fm]_/.test(id) ? (LANGUAGES[id[0]] ?? null) : null;

/** Preview, or Stop while `voice` is the one previewing. */
export const PreviewButton: React.FC<{ voice: string; disabled?: boolean }> = ({
  voice,
  disabled = false,
}) => {
  const { t } = useTranslation();
  const { active, preview, stop } = useVoicePreview();
  return active === voice ? (
    <Button variant="secondary" size="sm" onClick={stop}>
      {t("settings.speech.voice.stop")}
    </Button>
  ) : (
    <Button
      variant="secondary"
      size="sm"
      disabled={disabled}
      onClick={() => preview(voice)}
    >
      {t("settings.speech.voice.preview")}
    </Button>
  );
};

/** What `voice`'s preview is waiting for, or why it failed. */
export const PreviewNotice: React.FC<{ voice: string }> = ({ voice }) => {
  const { t } = useTranslation();
  const { getSetting } = useSettings();
  const { active, waiting, problem } = useVoicePreview();
  if (problem?.voice === voice) {
    return (
      <p className="text-xs text-red-500">
        {problem.problem.kind === "serverSlow"
          ? t("settings.speech.voice.serverSlow")
          : t("settings.speech.voice.previewFailed", {
              error: problem.problem.message,
            })}
      </p>
    );
  }
  if (active === voice && waiting && getSetting("tts_backend") === "http") {
    return (
      <p className="text-xs text-mid-gray">
        {t("settings.speech.voice.waitingForServer")}
      </p>
    );
  }
  return null;
};

interface VoiceRowProps {
  id: string;
  /** The catalog file; unset for a server voice or a file outside the catalog. */
  file?: ModelFile;
  active: boolean;
  /** Percent of the selected blend this voice makes up; unset for one voice. */
  share?: number;
  downloaded?: number;
  /** Off where the row's voice is not what the engine in use would read. */
  previewable?: boolean;
  onUse?: () => void;
  onDownload?: () => void;
  onCancel?: () => void;
  onDelete?: () => void;
}

/** One voice, as the Speech picker and the Models page both show it. */
export const VoiceRow: React.FC<VoiceRowProps> = ({
  id,
  file,
  previewable = true,
  ...props
}) => {
  const { t } = useTranslation();
  const megabytes = useMegabytes();
  const { active } = useVoicePreview();
  const language = file?.language ?? languageOf(id);
  const gender = languageOf(id) && id[1];
  let state: string | null = null;
  if (file) {
    state = file.installed
      ? t("settings.models.voices.installed")
      : t("settings.models.voices.notInstalled", {
          size: megabytes(file.size),
        });
  }
  const description = [
    language && t(`settings.models.voices.language.${language}`),
    gender &&
      t(
        gender === "f"
          ? "settings.models.voices.female"
          : "settings.models.voices.male",
      ),
    state,
  ]
    .filter(Boolean)
    .join(" · ");
  // A voice file must be here; the preview command reports a missing model.
  const canPreview = previewable && (file?.installed ?? true);

  return (
    <FileRow
      name={id}
      file={file}
      description={description}
      highlighted={active === id}
      actions={canPreview && <PreviewButton voice={id} />}
      notice={<PreviewNotice voice={id} />}
      {...props}
    />
  );
};

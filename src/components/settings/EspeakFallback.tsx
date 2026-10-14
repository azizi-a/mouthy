import React, { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { ToggleSwitch } from "../ui/ToggleSwitch";
import { useSettings } from "../../hooks/useSettings";
import { commands } from "@/bindings";

interface EspeakFallbackProps {
  descriptionMode?: "inline" | "tooltip";
  grouped?: boolean;
}

export const EspeakFallback: React.FC<EspeakFallbackProps> = ({
  descriptionMode = "tooltip",
  grouped = false,
}) => {
  const { t } = useTranslation();
  const { getSetting, updateSetting, isUpdating } = useSettings();
  const enabled = getSetting("tts_espeak_fallback") ?? false;
  // undefined until the first check returns.
  const [program, setProgram] = useState<string | null>();

  // Checked again on each toggle, so installing it needs no restart.
  useEffect(() => {
    let current = true;
    commands
      .findEspeakNg()
      .then((path) => {
        if (current) setProgram(path);
      })
      .catch(() => {});
    return () => {
      current = false;
    };
  }, [enabled]);

  return (
    <div>
      <ToggleSwitch
        checked={enabled}
        onChange={(value) => updateSetting("tts_espeak_fallback", value)}
        isUpdating={isUpdating("tts_espeak_fallback")}
        label={t("settings.speech.espeak.title")}
        description={t("settings.speech.espeak.description")}
        descriptionMode={descriptionMode}
        grouped={grouped}
      />
      {program !== undefined && (
        <p className="px-4 pb-2 text-xs text-mid-gray">
          {program
            ? t("settings.speech.espeak.found", { path: program })
            : t("settings.speech.espeak.notFound")}
        </p>
      )}
    </div>
  );
};

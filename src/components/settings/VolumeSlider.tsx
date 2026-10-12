import React from "react";
import { useTranslation } from "react-i18next";
import { Slider } from "../ui/Slider";
import { useSettings } from "../../hooks/useSettings";

interface VolumeSliderProps {
  descriptionMode?: "inline" | "tooltip";
  grouped?: boolean;
}

export const VolumeSlider: React.FC<VolumeSliderProps> = ({
  descriptionMode = "tooltip",
  grouped = false,
}) => {
  const { t } = useTranslation();
  const { getSetting, updateSetting, resetSetting, isUpdating } = useSettings();

  return (
    <Slider
      value={getSetting("tts_volume") ?? 1}
      onChange={(value) => updateSetting("tts_volume", value)}
      onReset={() => resetSetting("tts_volume")}
      isResetting={isUpdating("tts_volume")}
      min={0}
      max={1}
      label={t("settings.speech.volume.title")}
      description={t("settings.speech.volume.description")}
      descriptionMode={descriptionMode}
      grouped={grouped}
      formatValue={(value) => `${Math.round(value * 100)}%`}
    />
  );
};

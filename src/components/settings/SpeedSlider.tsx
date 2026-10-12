import React from "react";
import { useTranslation } from "react-i18next";
import { Slider } from "../ui/Slider";
import { useSettings } from "../../hooks/useSettings";

interface SpeedSliderProps {
  descriptionMode?: "inline" | "tooltip";
  grouped?: boolean;
}

export const SpeedSlider: React.FC<SpeedSliderProps> = ({
  descriptionMode = "tooltip",
  grouped = false,
}) => {
  const { t } = useTranslation();
  const { getSetting, updateSetting, resetSetting, isUpdating } = useSettings();

  return (
    <Slider
      value={getSetting("tts_speed") ?? 1}
      onChange={(value) => updateSetting("tts_speed", value)}
      onReset={() => resetSetting("tts_speed")}
      isResetting={isUpdating("tts_speed")}
      min={0.5}
      max={2}
      step={0.05}
      label={t("settings.speech.speed.title")}
      description={t("settings.speech.speed.description")}
      descriptionMode={descriptionMode}
      grouped={grouped}
      formatValue={(value) => `${value.toFixed(2)}×`}
    />
  );
};

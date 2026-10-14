import React from "react";
import { useTranslation } from "react-i18next";
import { Dropdown } from "../ui/Dropdown";
import { SettingContainer } from "../ui/SettingContainer";
import { useSettings } from "../../hooks/useSettings";
import type { TtsBackend } from "@/bindings";

interface TtsBackendSelectorProps {
  descriptionMode?: "inline" | "tooltip";
  grouped?: boolean;
}

export const TtsBackendSelector: React.FC<TtsBackendSelectorProps> = ({
  descriptionMode = "tooltip",
  grouped = false,
}) => {
  const { t } = useTranslation();
  const { getSetting, updateSetting, isUpdating } = useSettings();
  const backend: TtsBackend = getSetting("tts_backend") ?? "local";

  const options = [
    { value: "local", label: t("settings.speech.backend.options.local") },
    { value: "http", label: t("settings.speech.backend.options.http") },
  ];

  return (
    <SettingContainer
      title={t("settings.speech.backend.title")}
      description={t("settings.speech.backend.description")}
      descriptionMode={descriptionMode}
      grouped={grouped}
    >
      <Dropdown
        options={options}
        selectedValue={backend}
        onSelect={(value) => updateSetting("tts_backend", value as TtsBackend)}
        disabled={isUpdating("tts_backend")}
      />
    </SettingContainer>
  );
};

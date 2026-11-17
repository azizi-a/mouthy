import React from "react";
import { useTranslation } from "react-i18next";
import { Dropdown } from "../ui/Dropdown";
import { SettingContainer } from "../ui/SettingContainer";
import { useSettings } from "../../hooks/useSettings";
import type { ReadingLanguage } from "@/bindings";

interface ReadingLanguageSelectorProps {
  descriptionMode?: "inline" | "tooltip";
  grouped?: boolean;
}

export const ReadingLanguageSelector: React.FC<
  ReadingLanguageSelectorProps
> = ({ descriptionMode = "tooltip", grouped = false }) => {
  const { t } = useTranslation();
  const { getSetting, updateSetting, isUpdating } = useSettings();
  const language: ReadingLanguage = getSetting("tts_language") ?? "voice";

  const options = [
    { value: "voice", label: t("settings.speech.language.options.voice") },
    { value: "en", label: t("settings.speech.language.options.en") },
  ];

  return (
    <SettingContainer
      title={t("settings.speech.language.title")}
      description={t("settings.speech.language.description")}
      descriptionMode={descriptionMode}
      grouped={grouped}
    >
      <Dropdown
        options={options}
        selectedValue={language}
        onSelect={(value) =>
          updateSetting("tts_language", value as ReadingLanguage)
        }
        disabled={isUpdating("tts_language")}
      />
    </SettingContainer>
  );
};

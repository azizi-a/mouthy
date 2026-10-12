import React, { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Dropdown } from "../ui/Dropdown";
import { Input } from "../ui/Input";
import { ResetButton } from "../ui/ResetButton";
import { SettingContainer } from "../ui/SettingContainer";
import { useSettings } from "../../hooks/useSettings";
import { commands } from "@/bindings";

interface VoiceSelectorProps {
  descriptionMode?: "inline" | "tooltip";
  grouped?: boolean;
}

export const VoiceSelector: React.FC<VoiceSelectorProps> = React.memo(
  ({ descriptionMode = "tooltip", grouped = false }) => {
    const { t } = useTranslation();
    const { getSetting, updateSetting, resetSetting, isUpdating } =
      useSettings();
    const voice = getSetting("tts_voice") ?? "";
    const baseUrl = getSetting("tts_base_url");
    const [draft, setDraft] = useState(voice);
    const [voices, setVoices] = useState<string[]>([]);

    useEffect(() => setDraft(voice), [voice]);

    // Servers without a voice list keep the free-text field only.
    useEffect(() => {
      let current = true;
      setVoices([]);
      commands
        .fetchTtsVoices()
        .then((result) => {
          if (current && result.status === "ok") setVoices(result.data);
        })
        .catch(() => {});
      return () => {
        current = false;
      };
    }, [baseUrl]);

    const commit = (value: string) => {
      if (value !== voice) updateSetting("tts_voice", value);
    };

    return (
      <SettingContainer
        title={t("settings.speech.voice.title")}
        description={t("settings.speech.voice.description")}
        descriptionMode={descriptionMode}
        grouped={grouped}
      >
        <div className="flex items-center space-x-1">
          <Input
            type="text"
            value={draft}
            onChange={(event) => setDraft(event.target.value)}
            onBlur={() => commit(draft)}
            onKeyDown={(event) => {
              if (event.key === "Enter") event.currentTarget.blur();
            }}
            spellCheck={false}
            variant="compact"
            className="w-36"
            disabled={isUpdating("tts_voice")}
          />
          {voices.length > 0 && (
            <Dropdown
              options={voices.map((id) => ({ value: id, label: id }))}
              selectedValue={voice}
              onSelect={commit}
              placeholder={t("settings.speech.voice.choose")}
              disabled={isUpdating("tts_voice")}
            />
          )}
          <ResetButton
            onClick={() => resetSetting("tts_voice")}
            disabled={isUpdating("tts_voice")}
          />
        </div>
      </SettingContainer>
    );
  },
);

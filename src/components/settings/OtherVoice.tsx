import React, { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Dropdown } from "../ui/Dropdown";
import { Input } from "../ui/Input";
import { SettingContainer } from "../ui/SettingContainer";
import { useSettings } from "../../hooks/useSettings";

interface OtherVoiceProps {
  /** What the backend offers: voice files here, or the server's list. */
  voices: string[];
  /** Why the server's list could not be fetched, if it could not. */
  error?: string;
}

/** Free-text entry for a voice id or blend the list does not offer. */
export const OtherVoice: React.FC<OtherVoiceProps> = ({ voices, error }) => {
  const { t } = useTranslation();
  const { getSetting, updateSetting, isUpdating } = useSettings();
  const voice = getSetting("tts_voice") ?? "";
  const [draft, setDraft] = useState(voice);

  useEffect(() => setDraft(voice), [voice]);

  const commit = (value: string) => {
    const next = value.trim();
    setDraft(next);
    if (next !== voice) updateSetting("tts_voice", next);
  };

  return (
    <SettingContainer
      title={t("settings.speech.voice.other.title")}
      description={t("settings.speech.voice.other.description")}
      descriptionMode="inline"
      grouped={true}
      layout="stacked"
    >
      <div className="flex items-center gap-1">
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
          className="flex-1 min-w-0 font-mono"
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
      </div>
      {error && (
        <p className="mt-1 text-xs text-red-500">
          {t("settings.speech.voice.serverUnreachable", { error })}
        </p>
      )}
    </SettingContainer>
  );
};

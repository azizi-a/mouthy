import React, { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Input } from "../ui/Input";
import { ResetButton } from "../ui/ResetButton";
import { SettingContainer } from "../ui/SettingContainer";
import { useSettings } from "../../hooks/useSettings";

interface BackendUrlProps {
  descriptionMode?: "inline" | "tooltip";
  grouped?: boolean;
}

export const BackendUrl: React.FC<BackendUrlProps> = React.memo(
  ({ descriptionMode = "tooltip", grouped = false }) => {
    const { t } = useTranslation();
    const { getSetting, updateSetting, resetSetting, isUpdating } =
      useSettings();
    const url = getSetting("tts_base_url") ?? "";
    const [draft, setDraft] = useState(url);

    useEffect(() => setDraft(url), [url]);

    const commit = () => {
      if (draft !== url) updateSetting("tts_base_url", draft);
    };

    return (
      <SettingContainer
        title={t("settings.speech.backendUrl.title")}
        description={t("settings.speech.backendUrl.description")}
        descriptionMode={descriptionMode}
        grouped={grouped}
      >
        <div className="flex items-center space-x-1">
          <Input
            type="url"
            value={draft}
            onChange={(event) => setDraft(event.target.value)}
            onBlur={commit}
            onKeyDown={(event) => {
              if (event.key === "Enter") event.currentTarget.blur();
            }}
            spellCheck={false}
            variant="compact"
            className="w-64"
            disabled={isUpdating("tts_base_url")}
          />
          <ResetButton
            onClick={() => resetSetting("tts_base_url")}
            disabled={isUpdating("tts_base_url")}
          />
        </div>
      </SettingContainer>
    );
  },
);

import React from "react";
import { useTranslation } from "react-i18next";
import { ShortcutInput } from "../ShortcutInput";
import { SettingsGroup } from "../../ui/SettingsGroup";
import { OutputDeviceSelector } from "../OutputDeviceSelector";
import { AutostartToggle } from "../AutostartToggle";
import { StartHidden } from "../StartHidden";
import { ShowOverlay } from "../ShowOverlay";
import { BackendUrl } from "../BackendUrl";
import { TtsBackendSelector } from "../TtsBackendSelector";
import { VoiceSelector } from "../VoiceSelector";
import { EspeakFallback } from "../EspeakFallback";
import { SpeedSlider } from "../SpeedSlider";
import { VolumeSlider } from "../VolumeSlider";
import { useSettings } from "../../../hooks/useSettings";

export const GeneralSettings: React.FC = () => {
  const { t } = useTranslation();
  const { getSetting } = useSettings();
  const usesServer = (getSetting("tts_backend") ?? "local") === "http";
  return (
    <div className="max-w-3xl w-full mx-auto space-y-6">
      <SettingsGroup title={t("settings.general.title")}>
        <ShortcutInput shortcutId="speak" grouped={true} />
        <ShortcutInput shortcutId="speak_clipboard" grouped={true} />
        <AutostartToggle descriptionMode="tooltip" grouped={true} />
        <StartHidden descriptionMode="tooltip" grouped={true} />
        <ShowOverlay descriptionMode="tooltip" grouped={true} />
      </SettingsGroup>
      <SettingsGroup title={t("settings.speech.title")}>
        <TtsBackendSelector descriptionMode="tooltip" grouped={true} />
        {usesServer && <BackendUrl descriptionMode="tooltip" grouped={true} />}
        <VoiceSelector descriptionMode="tooltip" grouped={true} />
        {!usesServer && (
          <EspeakFallback descriptionMode="tooltip" grouped={true} />
        )}
        <SpeedSlider descriptionMode="tooltip" grouped={true} />
        <VolumeSlider descriptionMode="tooltip" grouped={true} />
      </SettingsGroup>
      <SettingsGroup title={t("settings.sound.title")}>
        <OutputDeviceSelector descriptionMode="tooltip" grouped={true} />
      </SettingsGroup>
    </div>
  );
};

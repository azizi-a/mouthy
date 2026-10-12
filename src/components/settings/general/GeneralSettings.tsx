import React from "react";
import { useTranslation } from "react-i18next";
import { ShortcutInput } from "../ShortcutInput";
import { SettingsGroup } from "../../ui/SettingsGroup";
import { OutputDeviceSelector } from "../OutputDeviceSelector";
import { AutostartToggle } from "../AutostartToggle";
import { StartHidden } from "../StartHidden";
import { ShowOverlay } from "../ShowOverlay";
import { BackendUrl } from "../BackendUrl";
import { VoiceSelector } from "../VoiceSelector";
import { SpeedSlider } from "../SpeedSlider";
import { VolumeSlider } from "../VolumeSlider";

export const GeneralSettings: React.FC = () => {
  const { t } = useTranslation();
  return (
    <div className="max-w-3xl w-full mx-auto space-y-6">
      <SettingsGroup title={t("settings.general.title")}>
        <ShortcutInput shortcutId="speak" grouped={true} />
        <AutostartToggle descriptionMode="tooltip" grouped={true} />
        <StartHidden descriptionMode="tooltip" grouped={true} />
        <ShowOverlay descriptionMode="tooltip" grouped={true} />
      </SettingsGroup>
      <SettingsGroup title={t("settings.speech.title")}>
        <BackendUrl descriptionMode="tooltip" grouped={true} />
        <VoiceSelector descriptionMode="tooltip" grouped={true} />
        <SpeedSlider descriptionMode="tooltip" grouped={true} />
        <VolumeSlider descriptionMode="tooltip" grouped={true} />
      </SettingsGroup>
      <SettingsGroup title={t("settings.sound.title")}>
        <OutputDeviceSelector descriptionMode="tooltip" grouped={true} />
      </SettingsGroup>
    </div>
  );
};

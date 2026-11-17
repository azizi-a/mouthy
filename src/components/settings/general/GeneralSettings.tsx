import React, { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { commands, type ShortcutHelp } from "@/bindings";
import { ShortcutInput } from "../ShortcutInput";
import { DesktopShortcuts } from "../DesktopShortcuts";
import { SettingsGroup } from "../../ui/SettingsGroup";
import { OutputDeviceSelector } from "../OutputDeviceSelector";
import { AutostartToggle } from "../AutostartToggle";
import { StartHidden } from "../StartHidden";
import { ShowOverlay } from "../ShowOverlay";
import { BackendUrl } from "../BackendUrl";
import { TtsBackendSelector } from "../TtsBackendSelector";
import { VoicePicker } from "../VoicePicker";
import { ReadingLanguageSelector } from "../ReadingLanguageSelector";
import { EspeakFallback } from "../EspeakFallback";
import { SpeedSlider } from "../SpeedSlider";
import { VolumeSlider } from "../VolumeSlider";
import { useSettings } from "../../../hooks/useSettings";

export const GeneralSettings: React.FC = () => {
  const { t } = useTranslation();
  const { getSetting } = useSettings();
  const usesServer = (getSetting("tts_backend") ?? "local") === "http";
  const keyboardImplementation = getSetting("keyboard_implementation");
  const [shortcutHelp, setShortcutHelp] = useState<ShortcutHelp | null>(null);

  // Asked again when the backend changes: Event Tap registers its own.
  useEffect(() => {
    let current = true;
    commands
      .shortcutHelp()
      .then((help) => {
        if (current) setShortcutHelp(help);
      })
      .catch(() => {});
    return () => {
      current = false;
    };
  }, [keyboardImplementation]);

  return (
    <div className="max-w-3xl w-full mx-auto space-y-6">
      <SettingsGroup title={t("settings.general.title")}>
        {shortcutHelp ? (
          <DesktopShortcuts command={shortcutHelp.command} grouped={true} />
        ) : (
          <>
            <ShortcutInput shortcutId="speak" grouped={true} />
            <ShortcutInput shortcutId="speak_clipboard" grouped={true} />
          </>
        )}
        <AutostartToggle descriptionMode="tooltip" grouped={true} />
        <StartHidden descriptionMode="tooltip" grouped={true} />
        <ShowOverlay descriptionMode="tooltip" grouped={true} />
      </SettingsGroup>
      <SettingsGroup title={t("settings.speech.title")}>
        <TtsBackendSelector descriptionMode="tooltip" grouped={true} />
        {usesServer && <BackendUrl descriptionMode="tooltip" grouped={true} />}
        <VoicePicker descriptionMode="tooltip" />
        {!usesServer && (
          <>
            <ReadingLanguageSelector descriptionMode="tooltip" grouped={true} />
            <EspeakFallback descriptionMode="tooltip" grouped={true} />
          </>
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

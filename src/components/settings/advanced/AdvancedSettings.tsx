import React from "react";
import { useTranslation } from "react-i18next";
import { SettingsGroup } from "../../ui/SettingsGroup";
import { ShowTrayIcon } from "../ShowTrayIcon";
import { ExperimentalToggle } from "../ExperimentalToggle";
import { useSettings } from "../../../hooks/useSettings";
import { KeyboardImplementationSelector } from "../debug/KeyboardImplementationSelector";

export const AdvancedSettings: React.FC = () => {
  const { t } = useTranslation();
  const { getSetting } = useSettings();
  const experimentalEnabled = getSetting("experimental_enabled") || false;

  return (
    <div className="max-w-3xl w-full mx-auto space-y-6">
      <SettingsGroup title={t("settings.advanced.groups.app")}>
        <ShowTrayIcon descriptionMode="tooltip" grouped={true} />
        <ExperimentalToggle descriptionMode="tooltip" grouped={true} />
      </SettingsGroup>

      {experimentalEnabled && (
        <SettingsGroup title={t("settings.advanced.groups.experimental")}>
          <KeyboardImplementationSelector
            descriptionMode="tooltip"
            grouped={true}
          />
        </SettingsGroup>
      )}
    </div>
  );
};

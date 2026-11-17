import React, { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { SettingContainer } from "../ui/SettingContainer";
import { commands } from "@/bindings";

interface DesktopShortcutsProps {
  /** What runs Mouthy, from `shortcutHelp`. */
  command: string;
  grouped?: boolean;
}

const ACTIONS = [
  {
    flag: "--speak-selection",
    label: "settings.general.shortcut.bindings.speak.name",
  },
  {
    flag: "--speak-clipboard",
    label: "settings.general.shortcut.bindings.speak_clipboard.name",
  },
  { flag: "--stop", label: "settings.general.shortcut.desktop.stop" },
] as const;

/** The commands to bind in the desktop's settings, where Mouthy's own shortcuts cannot work. */
export const DesktopShortcuts: React.FC<DesktopShortcutsProps> = ({
  command,
  grouped = false,
}) => {
  const { t } = useTranslation();
  // undefined until the first check returns.
  const [wlPaste, setWlPaste] = useState<string | null>();

  useEffect(() => {
    let current = true;
    commands
      .findWlPaste()
      .then((path) => {
        if (current) setWlPaste(path);
      })
      .catch(() => {});
    return () => {
      current = false;
    };
  }, []);

  return (
    <SettingContainer
      title={t("settings.general.shortcut.desktop.title")}
      description={t("settings.general.shortcut.desktop.description")}
      descriptionMode="inline"
      layout="stacked"
      grouped={grouped}
    >
      <div className="space-y-2">
        <dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 px-2 py-2 bg-mid-gray/10 border border-mid-gray/80 rounded-lg text-xs">
          {ACTIONS.map(({ flag, label }) => (
            <React.Fragment key={flag}>
              <dt className="text-mid-gray">{t(label)}</dt>
              <dd className="font-mono break-all select-all cursor-text">
                {`${command} ${flag}`}
              </dd>
            </React.Fragment>
          ))}
        </dl>
        <p className="text-xs text-mid-gray">
          {t("settings.general.shortcut.desktop.notRunning")}
        </p>
        <p className="text-xs text-mid-gray">
          {t("settings.general.shortcut.desktop.eventTap")}
        </p>
        <p className="text-xs text-mid-gray">
          {t("settings.general.shortcut.desktop.password")}
        </p>
        {wlPaste !== undefined && (
          <p className="text-xs text-mid-gray">
            {wlPaste
              ? t("settings.general.shortcut.desktop.wlPasteFound", {
                  path: wlPaste,
                })
              : t("settings.general.shortcut.desktop.wlPasteNotFound")}
          </p>
        )}
      </div>
    </SettingContainer>
  );
};

import React, { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { emitTo } from "@tauri-apps/api/event";
import { commands, type ModelFile, type VoicePart } from "@/bindings";
import { useModelFiles } from "../../hooks/useModelFiles";
import { useSettings } from "../../hooks/useSettings";
import { Button } from "../ui/Button";
import { ResetButton } from "../ui/ResetButton";
import { SettingContainer } from "../ui/SettingContainer";
import { ToggleSwitch } from "../ui/ToggleSwitch";
import { OtherVoice } from "./OtherVoice";
import { VoiceBlend } from "./VoiceBlend";
import { languageOf, VoiceRow } from "./VoiceRow";

const ENGLISH = ["britishEnglish", "americanEnglish"];
const OTHER_GROUP = "other";

type VoiceIds =
  | { status: "loading" }
  | { status: "ok"; ids: string[] }
  | { status: "error"; error: string };

interface Group {
  key: string;
  voices: string[];
}

/** `ids` grouped by language, English first, in the order they come. */
const groupByLanguage = (
  ids: string[],
  language: (id: string) => string | null,
): Group[] => {
  const groups = new Map<string, string[]>();
  for (const key of ENGLISH) groups.set(key, []);
  for (const id of ids) {
    const key = language(id) ?? OTHER_GROUP;
    groups.set(key, [...(groups.get(key) ?? []), id]);
  }
  const other = groups.get(OTHER_GROUP);
  groups.delete(OTHER_GROUP);
  if (other) groups.set(OTHER_GROUP, other);
  return [...groups]
    .filter(([, voices]) => voices.length > 0)
    .map(([key, voices]) => ({ key, voices }));
};

const Skeleton: React.FC = () => (
  <div className="px-4 py-3 space-y-3 animate-pulse">
    {[0, 1, 2].map((row) => (
      <div key={row} className="flex items-center justify-between gap-4">
        <div className="space-y-1.5">
          <div className="h-3.5 w-28 rounded bg-mid-gray/20" />
          <div className="h-3 w-44 rounded bg-mid-gray/15" />
        </div>
        <div className="h-6 w-28 rounded-lg bg-mid-gray/15" />
      </div>
    ))}
  </div>
);

interface VoicePickerProps {
  descriptionMode?: "inline" | "tooltip";
}

/** The Speech group's voice rows: in use, every voice by language, blend, other. */
export const VoicePicker: React.FC<VoicePickerProps> = ({
  descriptionMode = "tooltip",
}) => {
  const { t } = useTranslation();
  const { getSetting, updateSetting, resetSetting, isUpdating } = useSettings();
  const { list, downloaded, download, cancel } = useModelFiles();
  const usesServer = getSetting("tts_backend") === "http";
  const baseUrl = getSetting("tts_base_url");
  const voice = getSetting("tts_voice") ?? "";
  const [installedOnly, setInstalledOnly] = useState(false);
  const [showOther, setShowOther] = useState(false);
  const [voiceIds, setVoiceIds] = useState<VoiceIds>({ status: "loading" });

  const catalog = list?.files.filter((file) => file.kind === "voice") ?? [];
  const installedKey = usesServer
    ? ""
    : catalog
        .filter((file) => file.installed)
        .map((file) => file.id)
        .join();

  // The server's voices, or every voice file here, catalog or not.
  useEffect(() => {
    let current = true;
    setVoiceIds({ status: "loading" });
    commands
      .fetchTtsVoices()
      .then((result) => {
        if (!current) return;
        setVoiceIds(
          result.status === "ok"
            ? { status: "ok", ids: result.data }
            : { status: "error", error: result.error },
        );
      })
      .catch((error) => {
        if (current) setVoiceIds({ status: "error", error: String(error) });
      });
    return () => {
      current = false;
    };
  }, [usesServer, baseUrl, installedKey]);

  const ids = voiceIds.status === "ok" ? voiceIds.ids : [];
  const serverError =
    usesServer && voiceIds.status === "error" ? voiceIds.error : undefined;
  const serverEmpty =
    usesServer && voiceIds.status === "ok" && ids.length === 0;

  // The backend parses the setting; a string it cannot parse is one opaque voice.
  const parts: VoicePart[] =
    list && list.voice_parts.length > 0
      ? list.voice_parts
      : voice
        ? [{ id: voice, share: 1 }]
        : [];
  const blended = parts.length > 1;
  const shareOf = (id: string) => {
    const part = parts.find((p) => p.id === id);
    return blended && part ? Math.round(part.share * 100) : undefined;
  };
  const inUse = (id: string) => parts.some((part) => part.id === id);

  const fileOf = (id: string): ModelFile | undefined =>
    usesServer ? undefined : catalog.find((file) => file.id === id);

  const row = (id: string, key: string) => {
    const file = fileOf(id);
    return (
      <VoiceRow
        key={key}
        id={id}
        file={file}
        active={inUse(id)}
        share={shareOf(id)}
        downloaded={file ? downloaded(id) : undefined}
        onUse={() => updateSetting("tts_voice", id)}
        onDownload={() => download(id)}
        onCancel={() => cancel(id)}
      />
    );
  };

  const header = (
    <SettingContainer
      title={t("settings.speech.voice.title")}
      description={t("settings.speech.voice.description")}
      descriptionMode={descriptionMode}
      grouped={true}
    >
      <ResetButton
        onClick={() => resetSetting("tts_voice")}
        disabled={isUpdating("tts_voice")}
        ariaLabel={t("settings.speech.voice.reset")}
      />
    </SettingContainer>
  );

  const loading = !list || (usesServer && voiceIds.status === "loading");
  if (loading) {
    return (
      <>
        {header}
        <Skeleton />
      </>
    );
  }

  const listed = usesServer
    ? groupByLanguage(ids, languageOf)
    : groupByLanguage(
        catalog
          .filter((file) => !installedOnly || file.installed)
          .map((file) => file.id),
        (id) => fileOf(id)?.language ?? null,
      );
  const noVoiceInstalled =
    !usesServer && !catalog.some((file) => file.installed);
  const otherOpen = showOther || serverError !== undefined || serverEmpty;

  return (
    <>
      {header}
      {parts.map((part) => row(part.id, `in-use-${part.id}`))}
      {!usesServer && (noVoiceInstalled || list.missing_model) && (
        <div className="px-4 p-2 flex items-center justify-between gap-4">
          <p className="text-xs text-mid-gray">
            {list.missing_model
              ? t("settings.speech.voice.modelMissing")
              : t("settings.speech.voice.noneInstalled")}
          </p>
          <Button
            variant="primary-soft"
            size="sm"
            className="shrink-0"
            onClick={() => emitTo("main", "show-section", "models")}
          >
            {t("settings.speech.voice.openModels")}
          </Button>
        </div>
      )}
      {!usesServer && (
        <ToggleSwitch
          checked={installedOnly}
          onChange={setInstalledOnly}
          label={t("settings.speech.voice.installedOnly.title")}
          description={t("settings.speech.voice.installedOnly.description")}
          descriptionMode="tooltip"
          grouped={true}
        />
      )}
      {listed.length > 0 ? (
        <div className="max-h-[400px] overflow-y-auto">
          {listed.map((group) => (
            <div key={group.key}>
              <div className="sticky top-0 z-10 px-4 py-1.5 bg-background border-b border-mid-gray/20">
                <h4 className="text-xs font-medium text-mid-gray uppercase tracking-wide">
                  {group.key === OTHER_GROUP
                    ? t("settings.speech.voice.otherGroup")
                    : t(`settings.models.voices.language.${group.key}`)}
                </h4>
                {group.key !== OTHER_GROUP && !ENGLISH.includes(group.key) && (
                  <p className="text-xs text-mid-gray">
                    {t("settings.speech.voice.englishOnly")}
                  </p>
                )}
              </div>
              <div className="divide-y divide-mid-gray/20">
                {group.voices.map((id) => row(id, id))}
              </div>
            </div>
          ))}
        </div>
      ) : (
        !serverError && (
          <p className="px-4 py-3 text-xs text-mid-gray">
            {serverEmpty
              ? t("settings.speech.voice.serverEmpty")
              : t("settings.speech.voice.noneListed")}
          </p>
        )
      )}
      {otherOpen ? (
        <OtherVoice voices={ids} error={serverError} />
      ) : (
        <div className="px-4 p-2">
          <Button variant="ghost" size="sm" onClick={() => setShowOther(true)}>
            {t("settings.speech.voice.other.show")}
          </Button>
        </div>
      )}
      <VoiceBlend
        parts={parts}
        candidates={ids}
        onEditOther={() => setShowOther(true)}
      />
    </>
  );
};

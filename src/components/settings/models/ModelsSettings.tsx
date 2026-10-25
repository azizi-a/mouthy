import React from "react";
import { useTranslation } from "react-i18next";
import { openUrl } from "@tauri-apps/plugin-opener";
import type { ModelFile, ModelVariant } from "@/bindings";
import { useModelFiles } from "../../../hooks/useModelFiles";
import { useSettings } from "../../../hooks/useSettings";
import { Button } from "../../ui/Button";
import { PathDisplay } from "../../ui/PathDisplay";
import { SettingContainer } from "../../ui/SettingContainer";
import { SettingsGroup } from "../../ui/SettingsGroup";
import { FileRow, useMegabytes } from "../FileRow";
import { VoiceRow } from "../VoiceRow";

const LICENCE_URL = "https://www.apache.org/licenses/LICENSE-2.0";
const SOURCE_URL = "https://huggingface.co/onnx-community/Kokoro-82M-v1.0-ONNX";

export const ModelsSettings: React.FC = () => {
  const { t } = useTranslation();
  const megabytes = useMegabytes();
  const { getSetting, updateSetting } = useSettings();
  const { list, downloaded, download, cancel, remove, openFolder } =
    useModelFiles();

  if (!list) return null;

  const backend = getSetting("tts_backend");
  const variant = getSetting("tts_model_variant");
  const voice = getSetting("tts_voice");
  const models = list.files.filter((file) => file.kind === "model");
  const voices = list.files.filter((file) => file.kind === "voice");
  const shares = new Map<string, number>();
  for (const { id, share } of list.voice_parts) {
    shares.set(id, (shares.get(id) ?? 0) + share);
  }
  const blended = shares.size > 1;
  const activeFiles = [
    models.find((file) => file.id === variant),
    ...voices.filter((file) => shares.has(file.id)),
  ].filter((file): file is ModelFile => file !== undefined);
  // What the backend found missing, so voices outside the list count too.
  const needed = [
    models.find((file) => file.id === list.missing_model),
    ...voices.filter((file) => list.missing_voices.includes(file.id)),
  ].filter((file): file is ModelFile => file !== undefined);
  const unlistedVoices = list.missing_voices.filter(
    (id) => !voices.some((file) => file.id === id),
  );

  const status = () => {
    if (backend !== "local") {
      return {
        text: t("settings.models.status.server"),
        action: (
          <Button
            variant="primary-soft"
            size="sm"
            onClick={() => updateSetting("tts_backend", "local")}
          >
            {t("settings.models.status.useLocal")}
          </Button>
        ),
      };
    }
    const problems = [
      needed.length > 0 &&
        t("settings.models.status.missing", {
          files: needed.map((file) => file.id).join(", "),
        }),
      unlistedVoices.length > 0 &&
        t("settings.models.status.unlistedVoice", {
          voice: unlistedVoices.join(", "),
        }),
      list.load_error &&
        t("settings.models.status.failed", { error: list.load_error }),
    ].filter((problem): problem is string => Boolean(problem));
    if (problems.length === 0) {
      return {
        text: t("settings.models.status.ready", {
          model: variant,
          voice,
        }),
      };
    }
    const waiting = needed.filter((file) => !file.downloading);
    const bytes = waiting.reduce((sum, file) => sum + file.size, 0);
    return {
      text: problems.join(" "),
      action: needed.length > 0 && (
        <Button
          variant="primary-soft"
          size="sm"
          disabled={waiting.length === 0}
          onClick={() => waiting.forEach((file) => download(file.id))}
        >
          {t("settings.models.status.downloadMissing", {
            size: megabytes(bytes),
          })}
        </Button>
      ),
    };
  };
  const { text, action } = status();

  const shareOf = (file: ModelFile) => {
    const share = shares.get(file.id);
    return blended && share !== undefined ? Math.round(share * 100) : undefined;
  };

  const rowProps = (file: ModelFile) => ({
    file,
    active: activeFiles.includes(file),
    share: shareOf(file),
    downloaded: downloaded(file.id),
    onDownload: () => download(file.id),
    onCancel: () => cancel(file.id),
    onDelete: () => remove(file.id),
  });

  return (
    <div className="max-w-3xl w-full mx-auto space-y-6">
      <SettingsGroup title={t("settings.models.title")}>
        <SettingContainer
          title={t("settings.models.status.title")}
          description={text}
          descriptionMode="inline"
          grouped={true}
        >
          {action}
        </SettingContainer>
      </SettingsGroup>

      <SettingsGroup
        title={t("settings.models.model.title")}
        description={t("settings.models.model.description")}
      >
        {models.map((file) => (
          <FileRow
            key={file.id}
            name={file.id}
            description={`${megabytes(file.size)} · ${t(`settings.models.model.${file.id}`)}`}
            onUse={() =>
              updateSetting("tts_model_variant", file.id as ModelVariant)
            }
            {...rowProps(file)}
          />
        ))}
      </SettingsGroup>

      <SettingsGroup
        title={t("settings.models.voices.title")}
        description={t("settings.models.voices.description")}
      >
        {voices.map((file) => (
          <VoiceRow
            key={file.id}
            id={file.id}
            onUse={() => updateSetting("tts_voice", file.id)}
            previewable={backend === "local"}
            {...rowProps(file)}
          />
        ))}
      </SettingsGroup>

      <SettingsGroup title={t("settings.models.files.title")}>
        <SettingContainer
          title={t("settings.models.files.folder.title")}
          description={t("settings.models.files.folder.description")}
          descriptionMode="inline"
          grouped={true}
          layout="stacked"
        >
          <PathDisplay path={list.dir} onOpen={openFolder} />
        </SettingContainer>
        <SettingContainer
          title={t("settings.models.files.licence.title")}
          description={t("settings.models.files.licence.description")}
          descriptionMode="inline"
          grouped={true}
          layout="stacked"
        >
          <div className="flex gap-2">
            <Button
              variant="secondary"
              size="sm"
              onClick={() => openUrl(LICENCE_URL)}
            >
              {t("settings.models.files.licence.licence")}
            </Button>
            <Button
              variant="secondary"
              size="sm"
              onClick={() => openUrl(SOURCE_URL)}
            >
              {t("settings.models.files.licence.source")}
            </Button>
          </div>
        </SettingContainer>
      </SettingsGroup>
    </div>
  );
};

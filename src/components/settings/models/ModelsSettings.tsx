import React, { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { openUrl } from "@tauri-apps/plugin-opener";
import type { ModelFile, ModelVariant } from "@/bindings";
import { useModelFiles } from "../../../hooks/useModelFiles";
import { useSettings } from "../../../hooks/useSettings";
import { Button } from "../../ui/Button";
import { PathDisplay } from "../../ui/PathDisplay";
import { SettingContainer } from "../../ui/SettingContainer";
import { SettingsGroup } from "../../ui/SettingsGroup";

const LICENCE_URL = "https://www.apache.org/licenses/LICENSE-2.0";
const SOURCE_URL = "https://huggingface.co/onnx-community/Kokoro-82M-v1.0-ONNX";

export const ModelsSettings: React.FC = () => {
  const { t, i18n } = useTranslation();
  const { getSetting, updateSetting } = useSettings();
  const {
    list,
    voiceStatus,
    downloaded,
    download,
    cancel,
    remove,
    openFolder,
  } = useModelFiles();

  if (!list) return null;

  const backend = getSetting("tts_backend");
  const variant = getSetting("tts_model_variant");
  const voice = getSetting("tts_voice");
  const models = list.files.filter((file) => file.kind === "model");
  const voices = list.files.filter((file) => file.kind === "voice");
  const activeFiles = [
    models.find((file) => file.id === variant),
    voices.find((file) => file.id === voice),
  ].filter((file): file is ModelFile => file !== undefined);
  const needed = activeFiles.filter((file) => !file.installed);

  const megabytes = (bytes: number) =>
    t("settings.models.size", {
      size: (bytes / 1e6).toLocaleString(i18n.language, {
        maximumFractionDigits: bytes < 1e7 ? 1 : 0,
      }),
    });

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
    if (needed.length > 0) {
      const waiting = needed.filter((file) => !file.downloading);
      const bytes = waiting.reduce((sum, file) => sum + file.size, 0);
      return {
        text: t("settings.models.status.missing", {
          files: needed.map((file) => file.id).join(", "),
        }),
        action: (
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
    }
    if (voiceStatus?.kind === "failed") {
      return {
        text: t("settings.models.status.failed", {
          error: voiceStatus.message,
        }),
      };
    }
    return {
      text: t("settings.models.status.ready", {
        model: variant,
        voice,
      }),
    };
  };
  const { text, action } = status();

  const voiceDescription = ({ id, size }: ModelFile) =>
    [
      t(
        id.startsWith("b")
          ? "settings.models.voices.british"
          : "settings.models.voices.american",
      ),
      t(
        id[1] === "f"
          ? "settings.models.voices.female"
          : "settings.models.voices.male",
      ),
      megabytes(size),
    ].join(" · ");

  const row = (file: ModelFile, description: string, onUse: () => void) => (
    <FileRow
      key={file.id}
      file={file}
      description={description}
      active={activeFiles.includes(file)}
      downloaded={downloaded(file.id)}
      onUse={onUse}
      onDownload={() => download(file.id)}
      onCancel={() => cancel(file.id)}
      onDelete={() => remove(file.id)}
    />
  );

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
        {models.map((file) =>
          row(
            file,
            `${megabytes(file.size)} · ${t(`settings.models.model.${file.id}`)}`,
            () => updateSetting("tts_model_variant", file.id as ModelVariant),
          ),
        )}
      </SettingsGroup>

      <SettingsGroup
        title={t("settings.models.voices.title")}
        description={t("settings.models.voices.description")}
      >
        {voices.map((file) =>
          row(file, voiceDescription(file), () =>
            updateSetting("tts_voice", file.id),
          ),
        )}
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

interface FileRowProps {
  file: ModelFile;
  description: string;
  active: boolean;
  downloaded: number;
  onUse: () => void;
  onDownload: () => void;
  onCancel: () => void;
  onDelete: () => void;
}

const FileRow: React.FC<FileRowProps> = ({
  file,
  description,
  active,
  downloaded,
  onUse,
  onDownload,
  onCancel,
  onDelete,
}) => {
  const { t } = useTranslation();
  const percent = Math.min(100, Math.floor((downloaded * 100) / file.size));
  let progress: string | null = null;
  if (file.downloading) {
    progress =
      downloaded >= file.size
        ? t("settings.models.checking")
        : t("settings.models.downloading", { percent });
  } else if (!file.installed && file.partial > 0) {
    progress = t("settings.models.paused", { percent });
  }

  return (
    <div className="px-4 p-2 min-h-12 flex flex-col justify-center gap-1">
      <div className="flex items-center justify-between gap-4">
        <div className="min-w-0">
          <div className="flex items-center gap-2">
            <h3 className="text-sm font-medium">{file.id}</h3>
            {active && (
              <span
                className={`text-xs px-1.5 rounded ${file.installed ? "bg-logo-primary/20" : "bg-mid-gray/20"}`}
              >
                {file.installed
                  ? t("settings.models.inUse")
                  : t("settings.models.selected")}
              </span>
            )}
          </div>
          <p className="text-xs text-mid-gray">{description}</p>
        </div>
        <div className="flex items-center gap-1 shrink-0">
          {file.installed && !active && !file.downloading && (
            <Button variant="secondary" size="sm" onClick={onUse}>
              {t("settings.models.use")}
            </Button>
          )}
          {file.downloading ? (
            <Button variant="secondary" size="sm" onClick={onCancel}>
              {t("settings.models.cancel")}
            </Button>
          ) : (
            !file.installed && (
              <Button variant="primary-soft" size="sm" onClick={onDownload}>
                {file.partial > 0
                  ? t("settings.models.resume")
                  : t("settings.models.download")}
              </Button>
            )
          )}
          {!file.downloading && (file.installed || file.partial > 0) && (
            <DeleteButton onConfirm={onDelete} />
          )}
        </div>
      </div>
      {progress && (
        <div className="flex items-center gap-2">
          <div className="h-1.5 flex-1 rounded-full bg-mid-gray/20 overflow-hidden">
            <div
              className="h-full bg-logo-primary transition-[width]"
              style={{ width: `${percent}%` }}
            />
          </div>
          <span className="text-xs text-mid-gray tabular-nums">{progress}</span>
        </div>
      )}
      {file.error && !file.downloading && (
        <p className="text-xs text-red-500">
          {t("settings.models.failed", { error: file.error })}
        </p>
      )}
    </div>
  );
};

/** Deletes on a second click within three seconds. */
const DeleteButton: React.FC<{ onConfirm: () => void }> = ({ onConfirm }) => {
  const { t } = useTranslation();
  const [armed, setArmed] = useState(false);

  useEffect(() => {
    if (!armed) return;
    const timer = setTimeout(() => setArmed(false), 3000);
    return () => clearTimeout(timer);
  }, [armed]);

  return (
    <Button
      variant={armed ? "danger" : "danger-ghost"}
      size="sm"
      onClick={() => {
        if (armed) {
          setArmed(false);
          onConfirm();
        } else {
          setArmed(true);
        }
      }}
    >
      {armed ? t("settings.models.confirmDelete") : t("settings.models.delete")}
    </Button>
  );
};

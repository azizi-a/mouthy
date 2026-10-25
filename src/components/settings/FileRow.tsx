import React, { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import type { ModelFile } from "@/bindings";
import { Button } from "../ui/Button";

/** Formats bytes as megabytes, with one decimal below 10 MB. */
export const useMegabytes = () => {
  const { t, i18n } = useTranslation();
  return (bytes: number) =>
    t("settings.models.size", {
      size: (bytes / 1e6).toLocaleString(i18n.language, {
        maximumFractionDigits: bytes < 1e7 ? 1 : 0,
      }),
    });
};

interface FileRowProps {
  name: string;
  /** Unset for a voice the server offers: always there, never downloaded. */
  file?: ModelFile;
  description: string;
  active: boolean;
  /** Percent of the selected blend this voice makes up; unset for one voice. */
  share?: number;
  downloaded?: number;
  /** Buttons before Use, such as Preview. */
  actions?: React.ReactNode;
  highlighted?: boolean;
  /** A line under the row, such as why a preview failed. */
  notice?: React.ReactNode;
  onUse?: () => void;
  onDownload?: () => void;
  onCancel?: () => void;
  onDelete?: () => void;
}

/** A model or voice: its badge, its download state and what can be done with it. */
export const FileRow: React.FC<FileRowProps> = ({
  name,
  file,
  description,
  active,
  share,
  downloaded = 0,
  actions,
  highlighted = false,
  notice,
  onUse,
  onDownload,
  onCancel,
  onDelete,
}) => {
  const { t } = useTranslation();
  const megabytes = useMegabytes();
  const available = file?.installed ?? true;
  const downloading = file?.downloading ?? false;
  const percent = file
    ? Math.min(100, Math.floor((downloaded * 100) / file.size))
    : 0;
  let progress: string | null = null;
  if (file?.downloading) {
    progress =
      downloaded >= file.size
        ? t("settings.models.checking")
        : `${t("settings.models.downloading", { percent })} · ${t(
            "settings.models.left",
            { size: megabytes(file.size - downloaded) },
          )}`;
  } else if (file && !file.installed && file.partial > 0) {
    progress = t("settings.models.paused", { percent });
  }
  let badge: string;
  if (available) {
    badge =
      share === undefined
        ? t("settings.models.inUse")
        : t("settings.models.inUseShare", { percent: share });
  } else {
    badge =
      share === undefined
        ? t("settings.models.selected")
        : t("settings.models.selectedShare", { percent: share });
  }

  return (
    <div
      className={`px-4 p-2 min-h-12 flex flex-col justify-center gap-1 transition-colors ${highlighted ? "bg-logo-primary/[0.09]" : ""}`}
    >
      <div className="flex items-center justify-between gap-4">
        <div className="min-w-0 max-w-[360px]">
          <div className="flex items-center gap-2 min-w-0">
            <h3 className="text-sm font-medium truncate" title={name}>
              {name}
            </h3>
            {active && (
              <span
                className={`text-xs px-1.5 rounded shrink-0 ${available ? "bg-logo-primary/20" : "bg-mid-gray/20"}`}
              >
                {badge}
              </span>
            )}
          </div>
          {description && (
            <p className="text-xs text-mid-gray truncate">{description}</p>
          )}
        </div>
        <div className="flex items-center gap-1 shrink-0">
          {actions}
          {onUse &&
            available &&
            (!active || share !== undefined) &&
            !downloading && (
              <Button variant="secondary" size="sm" onClick={onUse}>
                {t("settings.models.use")}
              </Button>
            )}
          {file &&
            (file.downloading ? (
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
            ))}
          {onDelete &&
            file &&
            !file.downloading &&
            (file.installed || file.partial > 0) && (
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
      {file?.error && !file.downloading && (
        <p className="text-xs text-red-500">
          {t("settings.models.failed", { error: file.error })}
        </p>
      )}
      {notice}
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

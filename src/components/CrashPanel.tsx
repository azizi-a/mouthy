import React, { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { getVersion } from "@tauri-apps/api/app";
import { TriangleAlert } from "lucide-react";

interface CrashPanelProps {
  error: Error;
  componentStack: string;
}

/** Shown in place of the Settings window when it fails to render. */
const CrashPanel: React.FC<CrashPanelProps> = ({ error, componentStack }) => {
  const { t } = useTranslation();
  const [version, setVersion] = useState<string | null>(null);

  useEffect(() => {
    getVersion()
      .then(setVersion)
      .catch(() => {});
  }, []);

  // V8 starts a stack with the message; WebKit lists only the frames.
  const heading = `${error.name}: ${error.message}`;
  const stack = error.stack ?? "";
  const lines = version ? [`Mouthy ${version}`] : [];
  lines.push(
    navigator.userAgent,
    "",
    stack.startsWith(heading) ? stack : `${heading}\n${stack}`,
  );
  if (componentStack) lines.push("", `Component stack:${componentStack}`);
  const details = lines.join("\n");

  return (
    <div className="h-screen overflow-y-auto bg-background text-text p-6 flex flex-col gap-3">
      <div className="flex items-center gap-3">
        <TriangleAlert className="h-6 w-6 shrink-0 text-error" />
        <h1 className="text-lg font-semibold">{t("crash.title")}</h1>
      </div>
      <p className="text-sm text-mid-gray">{t("crash.hint")}</p>
      <p className="text-sm font-medium select-text break-words">
        {error.message}
      </p>
      <pre className="rounded-lg border border-mid-gray/30 bg-[var(--color-log-surface)] p-3 font-mono text-xs leading-relaxed select-text whitespace-pre-wrap break-words">
        {details}
      </pre>
    </div>
  );
};

export default CrashPanel;

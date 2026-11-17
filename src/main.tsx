import React from "react";
import ReactDOM from "react-dom/client";
import { platform } from "@tauri-apps/plugin-os";
import { ErrorBoundary } from "./components/ErrorBoundary";
import CrashPanel from "./components/CrashPanel";
import App from "./App";
import {
  applyTheme,
  getStoredTheme,
  syncThemeFromSettings,
} from "./lib/utils/theme";

// Read by the start-up probe in index.html.
document.documentElement.dataset.scriptStarted = "";

// Set platform before render so CSS can scope per-platform (e.g. scrollbar styles)
document.documentElement.dataset.platform = platform();

// Apply the last-known theme synchronously before render to avoid a flash of
// the wrong palette, then reconcile with the persisted setting once it loads.
applyTheme(getStoredTheme());
syncThemeFromSettings();

// Initialize i18n
import "./i18n";

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <ErrorBoundary
      context="Settings"
      fallback={(error, componentStack) => (
        <CrashPanel error={error} componentStack={componentStack} />
      )}
    >
      <App />
    </ErrorBoundary>
  </React.StrictMode>,
);

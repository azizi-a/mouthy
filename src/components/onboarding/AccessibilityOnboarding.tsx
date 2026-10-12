import { useEffect, useState, useCallback, useRef } from "react";
import { useTranslation } from "react-i18next";
import { platform } from "@tauri-apps/plugin-os";
import {
  checkAccessibilityPermission,
  requestAccessibilityPermission,
} from "tauri-plugin-macos-permissions-api";
import { toast } from "sonner";
import { commands } from "@/bindings";
import { useSettingsStore } from "@/stores/settingsStore";
import MouthyWordmark from "../icons/MouthyWordmark";
import { Keyboard, Check, Loader2 } from "lucide-react";

interface AccessibilityOnboardingProps {
  onComplete: () => void;
  preview?: boolean;
}

type PermissionStatus = "checking" | "needed" | "waiting" | "granted";

const initializeInput = () =>
  Promise.all([commands.initializeEnigo(), commands.initializeShortcuts()]);

const AccessibilityOnboarding: React.FC<AccessibilityOnboardingProps> = ({
  onComplete,
  preview = false,
}) => {
  const { t } = useTranslation();
  const refreshOutputDevices = useSettingsStore(
    (state) => state.refreshOutputDevices,
  );
  const [isMacOS, setIsMacOS] = useState<boolean | null>(null);
  const [accessibility, setAccessibility] =
    useState<PermissionStatus>("checking");
  const pollingRef = useRef<ReturnType<typeof setInterval> | null>(null);
  const timeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const errorCountRef = useRef<number>(0);
  const MAX_POLLING_ERRORS = 3;

  // Only macOS gates on a permission: Accessibility, to send the copy keystroke.
  const allGranted = isMacOS === false || accessibility === "granted";

  const completeOnboarding = useCallback(async () => {
    await refreshOutputDevices();
    timeoutRef.current = setTimeout(() => onComplete(), 300);
  }, [onComplete, refreshOutputDevices]);

  // Check platform and permission status on mount
  useEffect(() => {
    const onMacOS = platform() === "macos";
    setIsMacOS(onMacOS);

    // Debug previews are intentionally inert: show the permission request UI
    // without checking or changing operating-system permissions.
    if (preview) {
      setAccessibility(onMacOS ? "needed" : "granted");
      return;
    }

    if (!onMacOS) {
      onComplete();
      return;
    }

    const checkInitial = async () => {
      try {
        const granted = await checkAccessibilityPermission();

        if (granted) {
          try {
            await initializeInput();
          } catch (e) {
            console.warn("Failed to initialize after permission grant:", e);
          }
        }

        setAccessibility(granted ? "granted" : "needed");

        if (granted) {
          await completeOnboarding();
        }
      } catch (error) {
        console.error("Failed to check macOS permissions:", error);
        toast.error(t("onboarding.permissions.errors.checkFailed"));
        setAccessibility("needed");
      }
    };

    checkInitial();
  }, [completeOnboarding, onComplete, preview, t]);

  // Polling for the permission after the user clicks the button
  const startPolling = useCallback(() => {
    if (pollingRef.current) return;

    pollingRef.current = setInterval(async () => {
      try {
        const granted = await checkAccessibilityPermission();

        if (granted) {
          if (pollingRef.current) {
            clearInterval(pollingRef.current);
            pollingRef.current = null;
          }
          setAccessibility("granted");
          initializeInput().catch((e) => {
            console.warn("Failed to initialize after permission grant:", e);
          });
          await completeOnboarding();
        }

        // Reset error count on success
        errorCountRef.current = 0;
      } catch (error) {
        console.error("Error checking permissions:", error);
        errorCountRef.current += 1;

        if (errorCountRef.current >= MAX_POLLING_ERRORS) {
          // Stop polling after too many consecutive errors
          if (pollingRef.current) {
            clearInterval(pollingRef.current);
            pollingRef.current = null;
          }
          toast.error(t("onboarding.permissions.errors.checkFailed"));
        }
      }
    }, 1000);
  }, [completeOnboarding, t]);

  // Cleanup polling and timeouts on unmount
  useEffect(() => {
    return () => {
      if (pollingRef.current) {
        clearInterval(pollingRef.current);
      }
      if (timeoutRef.current) {
        clearTimeout(timeoutRef.current);
      }
    };
  }, []);

  const handleGrantAccessibility = async () => {
    if (preview) return;

    try {
      await requestAccessibilityPermission();
      setAccessibility("waiting");
      startPolling();
    } catch (error) {
      console.error("Failed to request accessibility permission:", error);
      toast.error(t("onboarding.permissions.errors.requestFailed"));
    }
  };

  // Still checking platform/initial permissions
  if (isMacOS === null || (isMacOS && accessibility === "checking")) {
    return (
      <div className="h-screen w-full flex items-center justify-center">
        <Loader2 className="w-8 h-8 animate-spin text-text/50" />
      </div>
    );
  }

  // All permissions granted - show success briefly
  if (allGranted) {
    return (
      <div className="h-screen w-full flex flex-col items-center justify-center gap-4">
        <div className="p-4 rounded-full bg-emerald-500/20">
          <Check className="w-12 h-12 text-emerald-400" />
        </div>
        <p className="text-lg font-medium text-text">
          {t("onboarding.permissions.allGranted")}
        </p>
      </div>
    );
  }

  // Show permissions request screen
  return (
    <div className="h-screen w-full flex flex-col p-6 gap-6 items-center justify-center">
      <div className="flex flex-col items-center gap-2">
        <MouthyWordmark className="text-5xl" />
      </div>

      <div className="max-w-md w-full flex flex-col items-center gap-4">
        <div className="text-center mb-2">
          <h2 className="text-xl font-semibold text-text mb-2">
            {t("onboarding.permissions.title")}
          </h2>
          <p className="text-text/70">
            {t("onboarding.permissions.description")}
          </p>
        </div>

        {/* Accessibility Permission Card */}
        <div className="w-full p-4 rounded-lg bg-white/5 border border-mid-gray/20">
          <div className="flex items-center gap-4">
            <div className="p-3 rounded-full bg-logo-primary/20 shrink-0">
              <Keyboard className="w-6 h-6 text-logo-primary" />
            </div>
            <div className="flex-1 min-w-0">
              <h3 className="font-medium text-text">
                {t("onboarding.permissions.accessibility.title")}
              </h3>
              <p className="text-sm text-text/60 mb-3">
                {t("onboarding.permissions.accessibility.description")}
              </p>
              {accessibility === "waiting" ? (
                <div className="flex items-center gap-2 text-text/50 text-sm">
                  <Loader2 className="w-4 h-4 animate-spin" />
                  {t("onboarding.permissions.waiting")}
                </div>
              ) : (
                <button
                  onClick={handleGrantAccessibility}
                  className="px-4 py-2 rounded-lg bg-logo-primary hover:bg-logo-primary/90 text-white text-sm font-medium transition-colors"
                >
                  {t("onboarding.permissions.grant")}
                </button>
              )}
            </div>
          </div>
        </div>
      </div>
    </div>
  );
};

export default AccessibilityOnboarding;

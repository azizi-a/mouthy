import { create } from "zustand";
import { subscribeWithSelector } from "zustand/middleware";
import { listen } from "@tauri-apps/api/event";
import { toast } from "sonner";
import type { AppSettings as Settings, AudioDevice } from "@/bindings";
import { commands } from "@/bindings";

interface SettingsStore {
  settings: Settings | null;
  defaultSettings: Settings | null;
  isLoading: boolean;
  isUpdating: Record<string, boolean>;
  outputDevices: AudioDevice[];

  // Actions
  initialize: () => Promise<void>;
  loadDefaultSettings: () => Promise<void>;
  updateSetting: <K extends keyof Settings>(
    key: K,
    value: Settings[K],
  ) => Promise<void>;
  resetSetting: (key: keyof Settings) => Promise<void>;
  refreshSettings: () => Promise<void>;
  refreshOutputDevices: () => Promise<void>;
  updateBinding: (id: string, binding: string) => Promise<void>;
  resetBinding: (id: string) => Promise<void>;
  getSetting: <K extends keyof Settings>(key: K) => Settings[K] | undefined;
  isUpdatingKey: (key: string) => boolean;
  // Internal state setters
  setSettings: (settings: Settings | null) => void;
  setDefaultSettings: (defaultSettings: Settings | null) => void;
  setLoading: (loading: boolean) => void;
  setUpdating: (key: string, updating: boolean) => void;
  setOutputDevices: (devices: AudioDevice[]) => void;
}

// Note: Default settings are now fetched from Rust via commands.getDefaultSettings()
// This ensures platform-specific defaults (like overlay_style and shortcuts) work correctly

const DEFAULT_AUDIO_DEVICE: AudioDevice = {
  index: "default",
  name: "Default",
  is_default: true,
};

const settingUpdaters: {
  [K in keyof Settings]?: (value: Settings[K]) => Promise<unknown>;
} = {
  start_hidden: (value) => commands.changeStartHiddenSetting(value as boolean),
  autostart_enabled: (value) =>
    commands.changeAutostartSetting(value as boolean),
  selected_output_device: (value) =>
    commands.setSelectedOutputDevice(
      (value as string) === "Default" || value === null
        ? "default"
        : (value as string),
    ),
  overlay_position: (value) =>
    commands.changeOverlayPositionSetting(value as string),
  debug_mode: (value) => commands.changeDebugModeSetting(value as boolean),
  log_level: (value) => commands.setLogLevel(value as any),
  app_language: (value) => commands.changeAppLanguageSetting(value as string),
  theme: (value) => commands.changeThemeSetting(value as string),
  experimental_enabled: (value) =>
    commands.changeExperimentalEnabledSetting(value as boolean),
  overlay_style: (value) => commands.changeOverlayStyleSetting(value as string),
  show_tray_icon: (value) =>
    commands.changeShowTrayIconSetting(value as boolean),
  onboarding_completed: (value) =>
    commands.changeOnboardingCompletedSetting(value as boolean),
  tts_base_url: (value) => commands.changeTtsBaseUrlSetting(value as string),
  // The Local engine rejects a malformed blend and stores its canonical form.
  tts_voice: async (value) => {
    const result = await commands.changeTtsVoiceSetting(value as string);
    if (result.status === "error") {
      toast.error(result.error);
      throw new Error(result.error);
    }
    await useSettingsStore.getState().refreshSettings();
  },
  tts_speed: (value) => commands.changeTtsSpeedSetting(value as number),
  tts_volume: (value) => commands.changeTtsVolumeSetting(value as number),
  tts_backend: (value) => commands.changeTtsBackendSetting(value as string),
  tts_model_variant: (value) =>
    commands.changeTtsModelVariantSetting(value as string),
  tts_espeak_fallback: (value) =>
    commands.changeTtsEspeakFallbackSetting(value as boolean),
};

export const useSettingsStore = create<SettingsStore>()(
  subscribeWithSelector((set, get) => ({
    settings: null,
    defaultSettings: null,
    isLoading: true,
    isUpdating: {},
    outputDevices: [],

    // Internal setters
    setSettings: (settings) => set({ settings }),
    setDefaultSettings: (defaultSettings) => set({ defaultSettings }),
    setLoading: (isLoading) => set({ isLoading }),
    setUpdating: (key, updating) =>
      set((state) => ({
        isUpdating: { ...state.isUpdating, [key]: updating },
      })),
    setOutputDevices: (outputDevices) => set({ outputDevices }),

    // Getters
    getSetting: (key) => get().settings?.[key],
    isUpdatingKey: (key) => get().isUpdating[key] || false,

    // Load settings from store
    refreshSettings: async () => {
      try {
        const result = await commands.getAppSettings();
        if (result.status === "ok") {
          const settings = result.data;
          const normalizedSettings: Settings = {
            ...settings,
            selected_output_device:
              settings.selected_output_device ?? "Default",
          };
          set({ settings: normalizedSettings, isLoading: false });
        } else {
          console.error("Failed to load settings:", result.error);
          set({ isLoading: false });
        }
      } catch (error) {
        console.error("Failed to load settings:", error);
        set({ isLoading: false });
      }
    },

    // Load output devices
    refreshOutputDevices: async () => {
      try {
        const result = await commands.getAvailableOutputDevices();
        if (result.status === "ok") {
          const devicesWithDefault = [
            DEFAULT_AUDIO_DEVICE,
            ...result.data.filter(
              (d) => d.name !== "Default" && d.name !== "default",
            ),
          ];
          set({ outputDevices: devicesWithDefault });
        } else {
          set({ outputDevices: [DEFAULT_AUDIO_DEVICE] });
        }
      } catch (error) {
        console.error("Failed to load output devices:", error);
        set({ outputDevices: [DEFAULT_AUDIO_DEVICE] });
      }
    },

    // Update a specific setting
    updateSetting: async <K extends keyof Settings>(
      key: K,
      value: Settings[K],
    ) => {
      const { settings, setUpdating } = get();
      const updateKey = String(key);
      const originalValue = settings?.[key];

      setUpdating(updateKey, true);

      try {
        set((state) => ({
          settings: state.settings ? { ...state.settings, [key]: value } : null,
        }));

        const updater = settingUpdaters[key];
        if (updater) {
          await updater(value);
        } else if (key !== "bindings") {
          console.warn(`No handler for setting: ${String(key)}`);
        }
      } catch (error) {
        console.error(`Failed to update setting ${String(key)}:`, error);
        if (settings) {
          set({ settings: { ...settings, [key]: originalValue } });
        }
      } finally {
        setUpdating(updateKey, false);
      }
    },

    // Reset a setting to its default value
    resetSetting: async (key) => {
      const { defaultSettings } = get();
      if (defaultSettings) {
        const defaultValue = defaultSettings[key];
        if (defaultValue !== undefined) {
          await get().updateSetting(key, defaultValue as any);
        }
      }
    },

    // Update a specific binding
    updateBinding: async (id, binding) => {
      const { settings, setUpdating } = get();
      const updateKey = `binding_${id}`;
      const originalBinding = settings?.bindings?.[id]?.current_binding;

      setUpdating(updateKey, true);

      try {
        // Optimistic update
        set((state) => ({
          settings: state.settings
            ? {
                ...state.settings,
                bindings: {
                  ...state.settings.bindings,
                  [id]: {
                    ...state.settings.bindings?.[id]!,
                    current_binding: binding,
                  },
                },
              }
            : null,
        }));

        const result = await commands.changeBinding(id, binding);

        // Check if the command executed successfully
        if (result.status === "error") {
          throw new Error(result.error);
        }

        // Check if the binding change was successful
        if (!result.data.success) {
          throw new Error(result.data.error || "Failed to update binding");
        }
      } catch (error) {
        console.error(`Failed to update binding ${id}:`, error);

        // Rollback on error; an unbound binding's original is ""
        if (originalBinding !== undefined && get().settings) {
          set((state) => ({
            settings: state.settings
              ? {
                  ...state.settings,
                  bindings: {
                    ...state.settings.bindings,
                    [id]: {
                      ...state.settings.bindings?.[id]!,
                      current_binding: originalBinding,
                    },
                  },
                }
              : null,
          }));
        }

        // Re-throw to let the caller know it failed
        throw error;
      } finally {
        setUpdating(updateKey, false);
      }
    },

    // Reset a specific binding
    resetBinding: async (id) => {
      const { setUpdating, refreshSettings } = get();
      const updateKey = `binding_${id}`;

      setUpdating(updateKey, true);

      try {
        const result = await commands.resetBinding(id);
        if (result.status === "error") {
          throw new Error(result.error);
        }
        if (!result.data.success) {
          throw new Error(result.data.error || "Failed to reset binding");
        }
        await refreshSettings();
      } catch (error) {
        console.error(`Failed to reset binding ${id}:`, error);
        // The caller shows it: a default can be refused when another binding has its keys.
        throw error;
      } finally {
        setUpdating(updateKey, false);
      }
    },

    // Load default settings from Rust
    loadDefaultSettings: async () => {
      try {
        const result = await commands.getDefaultSettings();
        if (result.status === "ok") {
          set({ defaultSettings: result.data });
        } else {
          console.error("Failed to load default settings:", result.error);
        }
      } catch (error) {
        console.error("Failed to load default settings:", error);
      }
    },

    // Initialize everything
    initialize: async () => {
      const { refreshSettings, loadDefaultSettings } = get();

      // Note: Output devices are NOT refreshed here. The frontend (App.tsx)
      // calls refreshOutputDevices after onboarding completes.
      await Promise.all([loadDefaultSettings(), refreshSettings()]);

      // Re-fetch settings when the backend changes them. The backend is the
      // source of truth.
      listen("settings-changed", () => {
        get().refreshSettings();
      });
    },
  })),
);

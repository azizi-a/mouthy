import { useEffect } from "react";
import { create } from "zustand";
import { listen } from "@tauri-apps/api/event";
import { commands } from "@/bindings";
import { useSettingsStore } from "../stores/settingsStore";

// Mirrors `SpeechStateChange` in actions.rs.
interface SpeechStateChange {
  state: "started" | "speaking" | "ended";
  preview: boolean;
  error?: string;
}

/** How long a server preview may go without audio before the row says so. */
const SERVER_WAIT_MS = 10_000;

export type PreviewProblem =
  | { kind: "failed"; message: string }
  | { kind: "serverSlow" };

interface VoicePreviewStore {
  /** The voice asked for, until its session starts or the request fails. */
  pending: string | null;
  /** The voice whose preview session is running. */
  playing: string | null;
  /** Whether that session's first audio has played. */
  speaking: boolean;
  problem: { voice: string; problem: PreviewProblem } | null;
}

const useStore = create<VoicePreviewStore>(() => ({
  pending: null,
  playing: null,
  speaking: false,
  problem: null,
}));

let serverWait: ReturnType<typeof setTimeout> | undefined;

const clearServerWait = () => {
  clearTimeout(serverWait);
  serverWait = undefined;
};

const onSpeechState = ({ state, preview, error }: SpeechStateChange) => {
  const { pending, playing } = useStore.getState();
  if (state === "started") {
    clearServerWait();
    // The backend has no voice in the event; a preview session is the one asked for last.
    const voice = preview ? pending : null;
    useStore.setState({ pending: null, playing: voice, speaking: false });
    const usesServer =
      useSettingsStore.getState().settings?.tts_backend === "http";
    if (voice && usesServer) {
      serverWait = setTimeout(() => {
        useStore.setState({
          problem: { voice, problem: { kind: "serverSlow" } },
        });
      }, SERVER_WAIT_MS);
    }
  } else if (state === "speaking") {
    clearServerWait();
    if (playing) {
      const { problem } = useStore.getState();
      useStore.setState({
        speaking: true,
        problem: problem?.voice === playing ? null : problem,
      });
    }
  } else {
    clearServerWait();
    const { problem } = useStore.getState();
    let next = problem;
    if (playing && error) {
      next = { voice: playing, problem: { kind: "failed", message: error } };
    } else if (problem?.problem.kind === "serverSlow") {
      // A wait that ended without an error is no longer worth showing.
      next = null;
    }
    // A preview replacing another ends it first, so `pending` stays.
    useStore.setState({ playing: null, speaking: false, problem: next });
  }
};

let listening = false;

const startListening = () => {
  if (listening) return;
  listening = true;
  listen<SpeechStateChange>("speech-state", ({ payload }) =>
    onSpeechState(payload),
  );
};

const preview = async (voice: string) => {
  useStore.setState({ pending: voice, problem: null });
  const result = await commands.previewVoice(voice);
  if (result.status === "error") {
    useStore.setState((state) => ({
      pending: state.pending === voice ? null : state.pending,
      problem: { voice, problem: { kind: "failed", message: result.error } },
    }));
  }
};

const stop = () => {
  useStore.setState({ pending: null, problem: null });
  return commands.stopSpeaking();
};

/** Voice preview state shared by every voice row, kept from `speech-state`. */
export const useVoicePreview = () => {
  useEffect(startListening, []);
  const state = useStore();
  return {
    /** The voice being previewed, from the request until the session ends. */
    active: state.pending ?? state.playing,
    /** Whether the active preview is still waiting for its first audio. */
    waiting:
      state.pending !== null || (state.playing !== null && !state.speaking),
    problem: state.problem,
    preview,
    stop,
  };
};

import { create } from "zustand";

export type SessionState = {
  recording: boolean;
  currentSessionId: string | null;
  setRecording: (recording: boolean) => void;
  setCurrentSession: (id: string | null) => void;
};

export const useSessionStore = create<SessionState>((set) => ({
  recording: false,
  currentSessionId: null,
  setRecording: (recording) => set({ recording }),
  setCurrentSession: (currentSessionId) => set({ currentSessionId }),
}));

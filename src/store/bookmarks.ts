import { create } from "zustand";

import {
  addBookmark,
  listBookmarks,
  removeBookmark,
  replaceBookmarks,
  type Bookmark,
} from "../ipc/commands";

export type BookmarksState = {
  items: Bookmark[];
  loading: boolean;
  error: string | null;
  refresh: () => Promise<void>;
  add: (name: string, frequencyHz: number) => Promise<void>;
  remove: (id: string) => Promise<void>;
  /// Wholesale replace (used by the "Load" menu entry).
  replaceAll: (bookmarks: Bookmark[]) => Promise<void>;
};

const formatError = (err: unknown): string => {
  if (err instanceof Error) return err.message;
  if (typeof err === "string") return err;
  try {
    return JSON.stringify(err);
  } catch {
    return "unknown error";
  }
};

export const useBookmarksStore = create<BookmarksState>((set) => ({
  items: [],
  loading: false,
  error: null,
  refresh: async () => {
    set({ loading: true, error: null });
    try {
      const items = await listBookmarks();
      set({ items, loading: false });
    } catch (err) {
      set({ loading: false, error: formatError(err) });
    }
  },
  add: async (name, frequencyHz) => {
    try {
      const created = await addBookmark(name, frequencyHz);
      set((state) => ({
        items: [...state.items, created].sort(
          (a, b) => a.createdAt - b.createdAt,
        ),
        error: null,
      }));
    } catch (err) {
      set({ error: formatError(err) });
    }
  },
  remove: async (id) => {
    try {
      await removeBookmark(id);
      set((state) => ({
        items: state.items.filter((b) => b.id !== id),
        error: null,
      }));
    } catch (err) {
      set({ error: formatError(err) });
    }
  },
  replaceAll: async (bookmarks) => {
    try {
      const saved = await replaceBookmarks(bookmarks);
      set({ items: saved, error: null });
    } catch (err) {
      set({ error: formatError(err) });
    }
  },
}));

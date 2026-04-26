import { useEffect, useRef, useState } from "react";
import { Channel } from "@tauri-apps/api/core";

import type { Bookmark } from "../../ipc/commands";
import { startScan } from "../../ipc/commands";
import { useBookmarksStore } from "../../store/bookmarks";
import { useCaptureStore } from "../../store/capture";
import { useRadioStore, type DemodMode } from "../../store/radio";
import { useReplayStore } from "../../store/replay";
import { useScannerStore } from "../../store/scanner";

type MenuKey = "file" | "view" | "bookmarks" | "capture" | "bands" | "settings";

type Band = { label: string; centerHz: number; scanRangeHz: number };

const BANDS: Band[] = [
  { label: "FM Broadcast", centerHz: 98_000_000, scanRangeHz: 10_000_000 },
  { label: "Aviation", centerHz: 121_500_000, scanRangeHz: 10_000_000 },
  { label: "Maritime VHF", centerHz: 156_800_000, scanRangeHz: 10_000_000 },
  { label: "2m Amateur", centerHz: 144_200_000, scanRangeHz: 10_000_000 },
  { label: "ISM 433", centerHz: 433_920_000, scanRangeHz: 10_000_000 },
  { label: "PMR446", centerHz: 446_000_000, scanRangeHz: 10_000_000 },
];

const BOOKMARK_FILE_VERSION = 1;
const BOOKMARK_EXPORT_NAME = "rail-bookmarks.json";

const formatFrequency = (hz: number): string => {
  if (hz >= 1_000_000) return `${(hz / 1_000_000).toFixed(3)} MHz`;
  if (hz >= 1_000) return `${(hz / 1_000).toFixed(3)} kHz`;
  return `${hz} Hz`;
};

/// Coerce anything we read from a user-supplied JSON into a safe
/// `Bookmark[]`. Accepts either `{ bookmarks: [...] }` (our own save
/// format) or a bare array. Unknown/bad entries are skipped.
const parseBookmarksFile = (raw: unknown): Bookmark[] => {
  const pool: unknown[] = Array.isArray(raw)
    ? raw
    : raw && typeof raw === "object" && Array.isArray((raw as { bookmarks?: unknown }).bookmarks)
      ? ((raw as { bookmarks: unknown[] }).bookmarks)
      : [];
  const out: Bookmark[] = [];
  for (const entry of pool) {
    if (!entry || typeof entry !== "object") continue;
    const e = entry as Record<string, unknown>;
    const freq = typeof e.frequencyHz === "number" ? e.frequencyHz : NaN;
    const name = typeof e.name === "string" ? e.name : "";
    if (!Number.isFinite(freq) || name.trim() === "") continue;
    out.push({
      id: typeof e.id === "string" && e.id.length > 0 ? e.id : crypto.randomUUID(),
      name: name.trim(),
      frequencyHz: Math.max(0, Math.round(freq)),
      ...(typeof e.mode === "string" && e.mode.length > 0 ? { mode: e.mode } : {}),
      ...(typeof e.bandwidthHz === "number" ? { bandwidthHz: e.bandwidthHz } : {}),
      createdAt:
        typeof e.createdAt === "number"
          ? e.createdAt
          : Math.floor(Date.now() / 1000),
    });
  }
  return out;
};

export const MenuBar = () => {
  const [open, setOpen] = useState<MenuKey | null>(null);
  const wrapRef = useRef<HTMLElement>(null);

  const frequencyHz = useRadioStore((s) => s.frequencyHz);
  const setFrequency = useRadioStore((s) => s.setFrequency);
  const setMode = useRadioStore((s) => s.setMode);
  const setBandwidth = useRadioStore((s) => s.setBandwidth);
  const streaming = useRadioStore((s) => s.streaming);
  const autoApplyMode = useRadioStore((s) => s.autoApplyMode);
  const setAutoApplyMode = useRadioStore((s) => s.setAutoApplyMode);
  const classifierEnabled = useRadioStore((s) => s.classifierEnabled);
  const setClassifierEnabled = useRadioStore((s) => s.setClassifierEnabled);
  const recordingAudio = useCaptureStore((s) => s.recordingAudio);
  const recordingIq = useCaptureStore((s) => s.recordingIq);
  const startAudio = useCaptureStore((s) => s.startAudio);
  const stopAudioWithSave = useCaptureStore((s) => s.stopAudioWithSave);
  const startIq = useCaptureStore((s) => s.startIq);
  const stopIqWithSave = useCaptureStore((s) => s.stopIqWithSave);
  const saveScreenshot = useCaptureStore((s) => s.saveScreenshot);
  const replayActive = useReplayStore((s) => s.active);
  const openReplayFile = useReplayStore((s) => s.openFile);
  const closeReplay = useReplayStore((s) => s.close);
  const scannerVisible = useScannerStore((s) => s.visible);
  const toggleScanner = useScannerStore((s) => s.toggleVisible);
  const items = useBookmarksStore((s) => s.items);
  const error = useBookmarksStore((s) => s.error);
  const add = useBookmarksStore((s) => s.add);
  const remove = useBookmarksStore((s) => s.remove);
  const replaceAll = useBookmarksStore((s) => s.replaceAll);
  const refresh = useBookmarksStore((s) => s.refresh);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  useEffect(() => {
    if (open === null) return;
    const onDocClick = (e: MouseEvent) => {
      if (!wrapRef.current?.contains(e.target as Node)) setOpen(null);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setOpen(null);
    };
    window.addEventListener("mousedown", onDocClick);
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("mousedown", onDocClick);
      window.removeEventListener("keydown", onKey);
    };
  }, [open]);

  const toggle = (key: MenuKey) => setOpen((cur) => (cur === key ? null : key));

  const serializeBookmarks = (): string =>
    JSON.stringify(
      { version: BOOKMARK_FILE_VERSION, bookmarks: items },
      null,
      2,
    );

  const downloadBookmarks = (body: string) => {
    const blob = new Blob([body], { type: "application/json" });
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = BOOKMARK_EXPORT_NAME;
    document.body.appendChild(a);
    a.click();
    a.remove();
    URL.revokeObjectURL(url);
  };

  const handleExport = async () => {
    const body = serializeBookmarks();
    const picker = (
      window as unknown as {
        showSaveFilePicker?: (opts: {
          suggestedName?: string;
          types?: Array<{
            description?: string;
            accept: Record<string, string[]>;
          }>;
        }) => Promise<{
          createWritable: () => Promise<{
            write: (data: string) => Promise<void>;
            close: () => Promise<void>;
          }>;
        }>;
      }
    ).showSaveFilePicker;

    if (typeof picker !== "function") {
      downloadBookmarks(body);
      setOpen(null);
      return;
    }
    try {
      const handle = await picker({
        suggestedName: BOOKMARK_EXPORT_NAME,
        types: [
          {
            description: "JSON",
            accept: { "application/json": [".json"] },
          },
        ],
      });
      const writable = await handle.createWritable();
      await writable.write(body);
      await writable.close();
    } catch (err) {
      if (err instanceof DOMException && err.name === "AbortError") {
        setOpen(null);
        return;
      }
      console.error("[RAIL] bookmark export failed:", err);
      downloadBookmarks(body);
    }
    setOpen(null);
  };

  const handleLoad = () => {
    const input = document.createElement("input");
    input.type = "file";
    input.accept = "application/json,.json";
    input.onchange = async () => {
      const file = input.files?.[0];
      if (!file) return;
      try {
        const text = await file.text();
        const parsed = parseBookmarksFile(JSON.parse(text));
        await replaceAll(parsed);
      } catch (err) {
        console.error("[RAIL] bookmark import failed:", err);
      }
    };
    input.click();
    setOpen(null);
  };

  const handleBandClick = async (band: Band) => {
    setOpen(null);
    if (!streaming) return;
    useRadioStore.getState().setFrequency(band.centerHz);
    if (!classifierEnabled) return;
    const startHz = Math.max(500_000, band.centerHz - band.scanRangeHz);
    const stopHz = band.centerHz + band.scanRangeHz;
    const scannerStore = useScannerStore.getState();
    scannerStore.setScanConfig({
      startHz,
      stopHz,
      stepHz: 200_000,
      dwellMs: 200,
      thresholdSnrDb: 10,
    });
    if (!scannerStore.visible) scannerStore.toggleVisible();
    const channel = new Channel<ArrayBuffer>();
    try {
      const reply = await startScan(
        { startHz, stopHz, stepHz: 200_000, dwellMs: 200, squelchSnrDb: null },
        channel,
      );
      useScannerStore.getState().beginScan(reply.frequenciesHz);
      const freqs = reply.frequenciesHz;
      channel.onmessage = (buffer: ArrayBuffer) => {
        const view = new DataView(buffer);
        const signalAvgDb = view.getFloat32(0, true);
        const noiseFloorDb = view.getFloat32(4, true);
        const idx = useScannerStore.getState().results.length;
        if (idx < freqs.length) {
          useScannerStore.getState().pushResult({ frequencyHz: freqs[idx], signalAvgDb, noiseFloorDb });
        }
      };
    } catch (err) {
      console.warn("[RAIL] band scan failed:", err);
    }
  };

  const handleAdd = () => {
    const name = window.prompt("Bookmark name", formatFrequency(frequencyHz));
    if (name === null) {
      setOpen(null);
      return;
    }
    const trimmed = name.trim();
    if (trimmed.length === 0) {
      setOpen(null);
      return;
    }
    const { mode, bandwidthHz } = useRadioStore.getState();
    void add(trimmed, frequencyHz, mode, bandwidthHz);
    setOpen(null);
  };

  return (
    <nav className="menu-bar" ref={wrapRef} aria-label="Application menu">
      <div className="menu-group">
        <button
          type="button"
          className={open === "file" ? "menu-top menu-top-open" : "menu-top"}
          aria-haspopup="menu"
          aria-expanded={open === "file"}
          onClick={() => toggle("file")}
        >
          File
        </button>
        {open === "file" && (
          <div className="menu-dropdown menu-dropdown-left" role="menu">
            <button
              type="button"
              role="menuitem"
              className="menu-item"
              onClick={() => {
                setOpen(null);
                void openReplayFile();
              }}
            >
              Open IQ file…
            </button>
            <button
              type="button"
              role="menuitem"
              className="menu-item"
              disabled={!replayActive}
              onClick={() => {
                setOpen(null);
                void closeReplay();
              }}
            >
              Close file
            </button>
          </div>
        )}
      </div>
      <div className="menu-group">
        <button
          type="button"
          className={open === "bands" ? "menu-top menu-top-open" : "menu-top"}
          aria-haspopup="menu"
          aria-expanded={open === "bands"}
          onClick={() => toggle("bands")}
        >
          Bands
        </button>
        {open === "bands" && (
          <div className="menu-dropdown menu-dropdown-left" role="menu">
            {BANDS.map((band) => (
              <button
                key={band.label}
                type="button"
                role="menuitem"
                className="menu-item"
                disabled={!streaming}
                onClick={() => { void handleBandClick(band); }}
              >
                {band.label}
                <span className="menu-item-freq">
                  {(band.centerHz / 1e6).toFixed(1)} MHz
                </span>
              </button>
            ))}
            <div className="menu-separator" role="separator" />
            <label className="menu-item menu-item-check">
              <input
                type="checkbox"
                checked={classifierEnabled}
                onChange={(e) => setClassifierEnabled(e.target.checked)}
              />
              Signal autodetection
            </label>
          </div>
        )}
      </div>
      <div className="menu-group">
        <button
          type="button"
          className={open === "settings" ? "menu-top menu-top-open" : "menu-top"}
          aria-haspopup="menu"
          aria-expanded={open === "settings"}
          onClick={() => toggle("settings")}
        >
          Settings
        </button>
        {open === "settings" && (
          <div className="menu-dropdown menu-dropdown-left" role="menu">
            <label className="menu-item menu-item-check">
              <input
                type="checkbox"
                checked={autoApplyMode}
                onChange={(e) => {
                  setAutoApplyMode(e.target.checked);
                }}
              />
              Auto-apply suggested mode
            </label>
          </div>
        )}
      </div>
      <div className="menu-group">
        <button
          type="button"
          className={open === "view" ? "menu-top menu-top-open" : "menu-top"}
          aria-haspopup="menu"
          aria-expanded={open === "view"}
          onClick={() => toggle("view")}
        >
          View
        </button>
        {open === "view" && (
          <div className="menu-dropdown menu-dropdown-left" role="menu">
            <button
              type="button"
              role="menuitem"
              className="menu-item"
              onClick={() => {
                toggleScanner();
                setOpen(null);
              }}
            >
              {scannerVisible ? "Hide Scanner" : "Show Scanner"}
            </button>
          </div>
        )}
      </div>
      <div className="menu-group">
        <button
          type="button"
          className={
            open === "bookmarks" ? "menu-top menu-top-open" : "menu-top"
          }
          aria-haspopup="menu"
          aria-expanded={open === "bookmarks"}
          onClick={() => toggle("bookmarks")}
        >
          Bookmarks
        </button>
        {open === "bookmarks" && (
          <div className="menu-dropdown" role="menu">
            <button
              type="button"
              role="menuitem"
              className="menu-item"
              onClick={() => void handleExport()}
            >
              Export…
            </button>
            <button
              type="button"
              role="menuitem"
              className="menu-item"
              onClick={handleLoad}
            >
              Load
            </button>
            <button
              type="button"
              role="menuitem"
              className="menu-item"
              onClick={handleAdd}
            >
              Add Bookmark
            </button>
            <div className="menu-separator" role="separator" />
            <div className="menu-section-label">Saved</div>
            {items.length === 0 ? (
              <div className="menu-empty">No bookmarks yet</div>
            ) : (
              <ul className="menu-bookmarks">
                {items.map((b) => (
                  <li key={b.id} className="menu-bookmark">
                    <button
                      type="button"
                      role="menuitem"
                      className="menu-bookmark-tune"
                      disabled={replayActive}
                      onClick={() => {
                        setFrequency(b.frequencyHz);
                        if (b.mode) setMode(b.mode as DemodMode);
                        if (b.bandwidthHz) setBandwidth(b.bandwidthHz);
                        setOpen(null);
                      }}
                      title={
                        replayActive
                          ? "Disabled during replay"
                          : `Tune to ${formatFrequency(b.frequencyHz)}${b.mode ? ` · ${b.mode}` : ""}`
                      }
                    >
                      <span className="menu-bookmark-name">{b.name}</span>
                      <span className="menu-bookmark-freq">
                        {formatFrequency(b.frequencyHz)}
                      </span>
                    </button>
                    <button
                      type="button"
                      className="menu-bookmark-delete"
                      aria-label={`Delete ${b.name}`}
                      title="Delete"
                      onClick={(e) => {
                        e.stopPropagation();
                        void remove(b.id);
                      }}
                    >
                      ×
                    </button>
                  </li>
                ))}
              </ul>
            )}
            {error && <div className="menu-error">{error}</div>}
          </div>
        )}
      </div>
      <div className="menu-group">
        <button
          type="button"
          className={
            open === "capture" ? "menu-top menu-top-open" : "menu-top"
          }
          aria-haspopup="menu"
          aria-expanded={open === "capture"}
          onClick={() => toggle("capture")}
        >
          Capture
          {(recordingAudio || recordingIq) && (
            <span
              className="menu-rec-dot"
              aria-label="Recording in progress"
            />
          )}
        </button>
        {open === "capture" && (
          <div className="menu-dropdown" role="menu">
            <button
              type="button"
              role="menuitem"
              className="menu-item"
              disabled={!streaming}
              onClick={() => {
                setOpen(null);
                void saveScreenshot();
              }}
            >
              Save screenshot…
            </button>
            <div className="menu-separator" role="separator" />
            <button
              type="button"
              role="menuitem"
              className="menu-item"
              disabled={!streaming && !recordingAudio}
              onClick={() => {
                setOpen(null);
                if (recordingAudio) void stopAudioWithSave();
                else void startAudio();
              }}
            >
              {recordingAudio ? "Stop audio recording" : "Start audio recording"}
            </button>
            <button
              type="button"
              role="menuitem"
              className="menu-item"
              disabled={!streaming && !recordingIq}
              onClick={() => {
                setOpen(null);
                if (recordingIq) void stopIqWithSave();
                else void startIq();
              }}
            >
              {recordingIq ? "Stop IQ recording" : "Start IQ recording"}
            </button>
          </div>
        )}
      </div>
    </nav>
  );
};

export default MenuBar;

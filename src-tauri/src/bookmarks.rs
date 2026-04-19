//! Named-frequency bookmarks persisted to disk.
//!
//! The store is a single JSON file living in Tauri's per-app config dir
//! (`app_config_dir()/bookmarks.json`). This keeps the data outside the
//! web-view sandbox and makes it user-inspectable / exportable. See
//! `docs/ARCHITECTURE.md` §6 for the IPC-contract conventions.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, Runtime};

use crate::error::RailError;

const FILE_VERSION: u32 = 1;
const FILE_NAME: &str = "bookmarks.json";

/// A single named frequency. `id` is monotonic within the process and
/// generated from `SystemTime::now()` nanoseconds as a hex string —
/// collision-free for a human-scale list, no extra dependency needed.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Bookmark {
    pub id: String,
    pub name: String,
    pub frequency_hz: u32,
    /// Unix epoch seconds. Cheap, sortable, no external crate.
    pub created_at: u64,
}

#[derive(Debug, Serialize, Deserialize)]
struct BookmarksFile {
    version: u32,
    bookmarks: Vec<Bookmark>,
}

impl Default for BookmarksFile {
    fn default() -> Self {
        Self {
            version: FILE_VERSION,
            bookmarks: Vec::new(),
        }
    }
}

/// Serialize access to the JSON file so concurrent commands don't trample
/// each other. Held as `tauri::State` — see `lib.rs`.
#[derive(Default)]
pub struct BookmarksStore {
    lock: Mutex<()>,
}

impl BookmarksStore {
    fn path<R: Runtime>(app: &AppHandle<R>) -> Result<PathBuf, RailError> {
        let dir = app
            .path()
            .app_config_dir()
            .map_err(|e| RailError::CaptureError(format!("app_config_dir: {e}")))?;
        Ok(dir.join(FILE_NAME))
    }

    fn load_file(path: &Path) -> Result<BookmarksFile, RailError> {
        match fs::read_to_string(path) {
            Ok(s) => {
                let file = serde_json::from_str::<BookmarksFile>(&s)
                    .map_err(|e| RailError::CaptureError(format!("bookmarks.json parse: {e}")))?;
                // Refuse to load formats newer than this build supports.
                // Without this guard, a Phase 7 schema bump would silently
                // drop unknown fields on the next save.
                if file.version > FILE_VERSION {
                    return Err(RailError::CaptureError(format!(
                        "bookmarks.json version {} is newer than supported ({})",
                        file.version, FILE_VERSION
                    )));
                }
                Ok(file)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(BookmarksFile::default()),
            Err(e) => Err(RailError::CaptureError(format!("bookmarks.json read: {e}"))),
        }
    }

    fn save_file(path: &Path, file: &BookmarksFile) -> Result<(), RailError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| RailError::CaptureError(format!("bookmarks dir: {e}")))?;
        }
        let body = serde_json::to_vec_pretty(file)
            .map_err(|e| RailError::CaptureError(format!("bookmarks serialize: {e}")))?;
        let tmp = path.with_extension("json.tmp");
        {
            let mut f = fs::File::create(&tmp)
                .map_err(|e| RailError::CaptureError(format!("bookmarks tmp create: {e}")))?;
            f.write_all(&body)
                .map_err(|e| RailError::CaptureError(format!("bookmarks tmp write: {e}")))?;
            f.sync_all()
                .map_err(|e| RailError::CaptureError(format!("bookmarks tmp sync: {e}")))?;
        }
        fs::rename(&tmp, path)
            .map_err(|e| RailError::CaptureError(format!("bookmarks rename: {e}")))
    }

    /// Return the full list, sorted by `createdAt` ascending (stable order
    /// the UI can rely on for chip layout).
    pub fn list<R: Runtime>(&self, app: &AppHandle<R>) -> Result<Vec<Bookmark>, RailError> {
        let _guard = self
            .lock
            .lock()
            .map_err(|_| RailError::CaptureError("bookmarks lock poisoned".into()))?;
        let file = Self::load_file(&Self::path(app)?)?;
        let mut out = file.bookmarks;
        out.sort_by_key(|b| b.created_at);
        Ok(out)
    }

    /// Append a new bookmark and persist.
    pub fn add<R: Runtime>(
        &self,
        app: &AppHandle<R>,
        name: String,
        frequency_hz: u32,
    ) -> Result<Bookmark, RailError> {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            return Err(RailError::InvalidParameter("bookmark name empty".into()));
        }
        let _guard = self
            .lock
            .lock()
            .map_err(|_| RailError::CaptureError("bookmarks lock poisoned".into()))?;
        let path = Self::path(app)?;
        let mut file = Self::load_file(&path)?;
        let now_nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| RailError::CaptureError(format!("clock error: {e}")))?;
        let bookmark = Bookmark {
            id: format!("{:x}", now_nanos.as_nanos()),
            name: trimmed.to_string(),
            frequency_hz,
            created_at: now_nanos.as_secs(),
        };
        file.bookmarks.push(bookmark.clone());
        Self::save_file(&path, &file)?;
        Ok(bookmark)
    }

    /// Remove a bookmark by id. Missing id is a no-op (idempotent).
    pub fn remove<R: Runtime>(&self, app: &AppHandle<R>, id: &str) -> Result<(), RailError> {
        let _guard = self
            .lock
            .lock()
            .map_err(|_| RailError::CaptureError("bookmarks lock poisoned".into()))?;
        let path = Self::path(app)?;
        let mut file = Self::load_file(&path)?;
        file.bookmarks.retain(|b| b.id != id);
        Self::save_file(&path, &file)
    }

    /// Wholesale replacement — used by the "Load" menu entry to swap the
    /// in-app list for the contents of a user-chosen JSON file. Returns
    /// the saved list (sorted) so the frontend can mirror state.
    pub fn replace<R: Runtime>(
        &self,
        app: &AppHandle<R>,
        bookmarks: Vec<Bookmark>,
    ) -> Result<Vec<Bookmark>, RailError> {
        let _guard = self
            .lock
            .lock()
            .map_err(|_| RailError::CaptureError("bookmarks lock poisoned".into()))?;
        let mut file = BookmarksFile {
            version: FILE_VERSION,
            bookmarks,
        };
        file.bookmarks.sort_by_key(|b| b.created_at);
        Self::save_file(&Self::path(app)?, &file)?;
        Ok(file.bookmarks)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_missing_returns_empty() {
        let dir = tempdir();
        let path = dir.join("bookmarks.json");
        let file = BookmarksStore::load_file(&path).unwrap();
        assert_eq!(file.version, FILE_VERSION);
        assert!(file.bookmarks.is_empty());
    }

    #[test]
    fn load_file_rejects_future_version() {
        let dir = tempdir();
        let path = dir.join("bookmarks.json");
        fs::write(&path, r#"{"version":2,"bookmarks":[]}"#).unwrap();
        match BookmarksStore::load_file(&path) {
            Err(RailError::CaptureError(msg)) => {
                assert!(msg.contains("newer than supported"), "got: {msg}");
            }
            other => panic!("expected CaptureError, got {other:?}"),
        }
    }

    #[test]
    fn save_then_load_round_trips() {
        let dir = tempdir();
        let path = dir.join("bookmarks.json");
        let b = Bookmark {
            id: "abc".into(),
            name: "BBC".into(),
            frequency_hz: 98_800_000,
            created_at: 1_700_000_000,
        };
        let file = BookmarksFile {
            version: FILE_VERSION,
            bookmarks: vec![b.clone()],
        };
        BookmarksStore::save_file(&path, &file).unwrap();
        let loaded = BookmarksStore::load_file(&path).unwrap();
        assert_eq!(loaded.bookmarks.len(), 1);
        assert_eq!(loaded.bookmarks[0].name, "BBC");
        assert_eq!(loaded.bookmarks[0].frequency_hz, 98_800_000);
    }

    fn tempdir() -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!(
            "rail-bookmarks-test-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&p).unwrap();
        p
    }
}

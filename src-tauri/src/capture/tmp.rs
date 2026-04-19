//! Temp-file helpers for the hybrid streaming-capture flow.
//!
//! All audio / IQ recordings stream into `app_data_dir()/capture-tmp/`
//! while in progress. When the user stops a recording, the frontend
//! shows a native Save As dialog and then asks the backend to move the
//! temp file(s) to the chosen location. If the user cancels, the temp
//! files are unlinked via [`discard_capture`](crate::ipc::commands).

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use tauri::{AppHandle, Manager, Runtime};

use crate::error::RailError;

/// Subfolder under `app_data_dir()` where in-progress captures live.
const TMP_SUBDIR: &str = "capture-tmp";

/// Resolve (and create) `app_data_dir()/capture-tmp/`.
pub fn tmp_dir<R: Runtime>(app: &AppHandle<R>) -> Result<PathBuf, RailError> {
    let base = app
        .path()
        .app_data_dir()
        .map_err(|e| RailError::CaptureError(format!("app_data_dir: {e}")))?;
    let dir = base.join(TMP_SUBDIR);
    std::fs::create_dir_all(&dir).map_err(|e| RailError::CaptureError(format!("tmp dir: {e}")))?;
    Ok(dir)
}

/// Build a new path inside the tmp dir with the given extension. The
/// base name is a nanosecond timestamp so two captures in flight at
/// once never collide.
pub fn new_tmp_path<R: Runtime>(app: &AppHandle<R>, ext: &str) -> Result<PathBuf, RailError> {
    let dir = tmp_dir(app)?;
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    Ok(dir.join(format!("rail-{nanos:x}.{ext}")))
}

/// Rename with cross-volume fallback (copy + remove). Plain `rename`
/// fails with `EXDEV` when src and dst live on different drives, which
/// is common on Windows when the user picks a Save location outside
/// the system drive.
pub fn move_file(src: &Path, dst: &Path) -> Result<(), RailError> {
    if let Some(parent) = dst.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| RailError::CaptureError(format!("dst dir: {e}")))?;
    }
    match std::fs::rename(src, dst) {
        Ok(()) => Ok(()),
        Err(_) => {
            std::fs::copy(src, dst).map_err(|e| RailError::CaptureError(format!("copy: {e}")))?;
            std::fs::remove_file(src).ok();
            Ok(())
        }
    }
}

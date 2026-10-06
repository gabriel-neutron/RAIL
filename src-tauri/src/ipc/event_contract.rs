//! The single emit seam for the JSON event bus (Rust → React).
//!
//! Every payload declared in `shared/ipc_events.json` implements
//! [`IpcEvent`], which carries its wire name; [`Emit`] is then supplied
//! for free by the blanket impl below. This is the only place
//! [`tauri::Emitter::emit`] is called. See `docs/ARCHITECTURE.md` §3.2.

use serde::Serialize;
use tauri::{AppHandle, Emitter, Runtime};

use crate::error::RailError;

/// A payload that travels on the named JSON event bus under `NAME`.
///
/// Implementations are generated from `shared/ipc_events.json`; do not
/// implement this by hand.
pub trait IpcEvent: Serialize {
    /// The wire name of the event (kebab-case), as the frontend listens for it.
    const NAME: &'static str;
}

/// Sends a payload to every frontend window.
pub trait Emit {
    /// Emit `self` on its wire name, wrapping any transport failure in
    /// [`RailError::StreamError`].
    fn emit<R: Runtime>(&self, app: &AppHandle<R>) -> Result<(), RailError>;
}

impl<T: IpcEvent> Emit for T {
    fn emit<R: Runtime>(&self, app: &AppHandle<R>) -> Result<(), RailError> {
        app.emit(T::NAME, self)
            .map_err(|e| RailError::StreamError(format!("emit {}: {e}", T::NAME)))
    }
}

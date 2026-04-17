//! RAIL backend entry point.
//!
//! Wires up module namespaces and registers Tauri commands.
//! See `docs/ARCHITECTURE.md` for module boundaries and IPC contract.

pub mod capture;
pub mod dsp;
pub mod error;
pub mod hardware;
pub mod ipc;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let _ = env_logger::try_init();

    let builder = ipc::commands::register(tauri::Builder::default());
    builder
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

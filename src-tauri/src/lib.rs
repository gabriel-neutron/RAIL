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

    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            ipc::commands::ping,
            ipc::commands::check_device,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

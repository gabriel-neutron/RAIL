//! RAIL backend entry point.
//!
//! Wires up module namespaces and registers Tauri commands.
//! See `docs/ARCHITECTURE.md` for module boundaries and IPC contract.

pub mod bookmarks;
pub mod capture;
pub mod dsp;
pub mod error;
pub mod hardware;
pub mod ipc;
pub mod perf_emit;
pub mod replay;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let _ = env_logger::try_init();

    let builder = tauri::Builder::default().plugin(tauri_plugin_dialog::init());
    let builder = ipc::commands::register(builder);
    if let Err(err) = builder.run(tauri::generate_context!()) {
        log::error!("tauri runtime exited with error: {err}");
        std::process::exit(1);
    }
}

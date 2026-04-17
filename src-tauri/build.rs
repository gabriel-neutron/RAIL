//! Build script: Tauri codegen, librtlsdr linkage, and (on Windows) DLL
//! staging next to the output binary.
//!
//! Link search:
//! - If `LIBRTLSDR_LIB_DIR` is set, use it verbatim.
//! - Otherwise default to `../vendor/librtlsdr-win-x64/` relative to the
//!   crate. See that folder's `README.md` for how to populate it.
//!
//! Runtime DLLs (`rtlsdr.dll`, `pthreadVC2.dll`, `msvcr100.dll`) are
//! copied into the cargo target profile dir so `cargo tauri dev` just
//! works without the user touching `PATH`. See `docs/TECH_STACK.md` §4.

use std::{
    env, fs,
    path::{Path, PathBuf},
};

fn main() {
    let lib_dir = resolve_lib_dir();
    if let Some(dir) = lib_dir.as_ref() {
        println!("cargo:rustc-link-search=native={}", dir.display());
    }
    println!("cargo:rustc-link-lib=rtlsdr");
    println!("cargo:rerun-if-env-changed=LIBRTLSDR_LIB_DIR");

    if cfg!(target_os = "windows") {
        if let Some(dir) = lib_dir.as_ref() {
            stage_windows_runtime_dlls(dir);
        } else {
            println!("cargo:warning=librtlsdr directory not found; set LIBRTLSDR_LIB_DIR or drop prebuilts under ../vendor/librtlsdr-win-x64 (see its README)");
        }
    }

    tauri_build::build()
}

fn resolve_lib_dir() -> Option<PathBuf> {
    if let Ok(raw) = env::var("LIBRTLSDR_LIB_DIR") {
        let p = PathBuf::from(raw);
        return if p.is_dir() { Some(p) } else { None };
    }
    let crate_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").ok()?);
    let default = crate_dir.join("..").join("vendor").join("librtlsdr-win-x64");
    if default.join("rtlsdr.lib").is_file() {
        Some(default.canonicalize().ok()?)
    } else {
        None
    }
}

fn stage_windows_runtime_dlls(lib_dir: &Path) {
    let Some(target_dir) = locate_target_profile_dir() else {
        println!("cargo:warning=could not locate target profile dir; skipping DLL copy");
        return;
    };
    for dll in ["rtlsdr.dll", "pthreadVC2.dll", "msvcr100.dll"] {
        let src = lib_dir.join(dll);
        let dst = target_dir.join(dll);
        if !src.is_file() {
            println!("cargo:warning=missing runtime DLL: {}", src.display());
            continue;
        }
        println!("cargo:rerun-if-changed={}", src.display());
        if let Err(e) = fs::copy(&src, &dst) {
            println!(
                "cargo:warning=failed to copy {} -> {}: {e}",
                src.display(),
                dst.display()
            );
        }
    }
}

/// `OUT_DIR` is like `<crate>/target/<profile>/build/<pkg>-<hash>/out`.
/// Walk up four levels to land on `<crate>/target/<profile>`.
fn locate_target_profile_dir() -> Option<PathBuf> {
    let out = PathBuf::from(env::var("OUT_DIR").ok()?);
    out.ancestors().nth(3).map(PathBuf::from)
}

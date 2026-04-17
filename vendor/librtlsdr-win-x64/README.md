# librtlsdr Windows x64 prebuilts

This folder holds the Windows x64 runtime required by the Rust backend to link
and load `librtlsdr`. The binaries themselves are **not checked in** (see
root `.gitignore`): they are redistributable but out of scope for this repo.

## Expected files

| File             | Role                                   |
| ---------------- | -------------------------------------- |
| `rtlsdr.lib`     | Import library used by the Rust linker |
| `rtlsdr.dll`     | Runtime DLL loaded by the app          |
| `pthreadVC2.dll` | `librtlsdr` transitive dependency      |
| `msvcr100.dll`   | MSVC 2010 runtime required by the DLL  |

## Where to get them

Official upstream build:
<https://ftp.osmocom.org/binaries/windows/rtl-sdr/rtl-sdr-64bit-20230830.zip>

Extract the contents of the zip's `rtl-sdr-64bit-.../` (or `Release/x64/`)
folder directly into this directory.

## How they are consumed

`src-tauri/build.rs` resolves the library directory in this order:

1. `LIBRTLSDR_LIB_DIR` environment variable, if set.
2. `vendor/librtlsdr-win-x64/` (this folder).

It then emits the cargo linker flags and, on Windows, copies the three DLLs
next to the built executable so `cargo tauri dev` runs without touching
`PATH`.

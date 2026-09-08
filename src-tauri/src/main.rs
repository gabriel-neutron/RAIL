// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
//! RAIL desktop binary.
//!
//! Thin launcher: all backend wiring lives in the `rail_lib` crate root.

fn main() {
    rail_lib::run()
}

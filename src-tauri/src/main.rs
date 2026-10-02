//! guft desktop entry point. Everything lives in the library so the same code runs on a phone.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    guft_lib::run();
}

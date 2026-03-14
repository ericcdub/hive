// Copyright (c) 2026 Eric Chubb
// Licensed under the MIT License

//! # Hive - Windows Registry Editor
//!
//! A modern, SQLite-first Windows Registry editor built with Rust and egui.
//!
//! ## Features
//!
//! - Browse and edit the Windows Registry
//! - SQLite-first architecture for safe editing
//! - Preview changes before committing
//! - Search across the entire registry
//! - Bookmarks with colors and notes
//! - Import/export .reg files
//!
//! ## Architecture
//!
//! - `app.rs` - Main application state and UI
//! - `rust-hive` - Registry access library (separate crate)

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;

use app::RegistryEditorApp;

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1200.0, 800.0])
            .with_min_inner_size([600.0, 400.0])
            .with_title("Hive - Registry Editor"),
        ..Default::default()
    };

    eframe::run_native(
        "Hive",
        options,
        Box::new(|cc| Ok(Box::new(RegistryEditorApp::new(cc)))),
    )
}

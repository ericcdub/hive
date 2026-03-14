// Copyright (c) 2026 Eric Chubb
// Licensed under the MIT License

//! # Hive - Registry Editor Application
//!
//! This is the main UI module for the Hive Windows Registry Editor.
//! It uses the `egui` immediate-mode GUI library via `eframe`.
//!
//! ## Architecture Overview
//!
//! The app follows a SQLite-first pattern:
//! - All data is read from a local SQLite database (via rust-hive)
//! - Changes are saved to SQLite first, then optionally synced to the registry
//! - This allows for preview, undo, and batch operations
//!
//! ## File Organization
//!
//! This file is organized into logical sections:
//!
//! | Lines | Section | Description |
//! |-------|---------|-------------|
//! | 1-90 | Module docs & imports | Documentation and use statements |
//! | 90-140 | UI State Enums | `Panel`, `SyncMode`, `EditDialog` |
//! | 140-250 | RegistryEditorApp | Main app struct and initialization |
//! | 250-420 | Menu Bar | `show_menu_bar()` with File/Edit/View/Sync menus |
//! | 420-650 | Left Panel | Tree, search, bookmarks tab container |
//! | 650-970 | Search Panel | Search UI and results display |
//! | 970-1100 | Bookmarks Panel | Bookmark list and management |
//! | 1100-1170 | Pending Changes | Shows pending registry changes |
//! | 1170-1350 | Values Panel | Registry values display and editing |
//! | 1350-2200 | Dialogs | All modal dialog implementations |
//! | 2200-2350 | Helper Functions | `export_reg_file()`, `import_reg_file()`, etc. |
//! | 2350-2452 | eframe::App impl | Main `update()` loop and keyboard shortcuts |
//!
//! ## Rust Concepts Used
//!
//! ### Immediate Mode GUI (egui)
//!
//! Unlike traditional retained-mode GUIs where you create widgets once,
//! immediate mode GUIs redraw everything each frame:
//!
//! ```rust
//! // This runs 60+ times per second:
//! fn update(&mut self, ctx: &egui::Context) {
//!     if ui.button("Click me").clicked() {
//!         // Handle click
//!     }
//! }
//! ```
//!
//! ### The `eframe::App` Trait
//!
//! Our app implements `eframe::App`, which requires an `update()` method
//! that's called every frame to render the UI.
//!
//! ## Future Refactoring
//!
//! This file could be split into:
//! - `ui/panels.rs` - Tree, search, bookmarks panels
//! - `ui/dialogs.rs` - Modal dialog implementations
//! - `ui/values.rs` - Values panel
//! - `ui/menu.rs` - Menu bar
//!
//! Each would contain `impl RegistryEditorApp { ... }` blocks.

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// IMPORTS
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

// Rust Hive library imports
use rust_hive::bookmarks::{Bookmark, BookmarkColor};
use rust_hive::registry::{self, RegValue, RegistryValue, RootKey};
use rust_hive::search::{MatchType, SearchOptions, SearchResult, SearchState};
use rust_hive::sync::{PendingChange, SyncConflict, SyncStore};

// External crate imports
use eframe::egui;
use std::sync::atomic::Ordering;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// UI STATE ENUMS
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Which panel is currently shown in the left sidebar.
///
/// The left sidebar has multiple "tabs" that show different content:
/// - **Tree**: The main registry key tree navigator
/// - **Search**: Full-text search across registry
/// - **Bookmarks**: User's saved bookmarks
/// - **PendingChanges**: Changes waiting to sync to registry
#[derive(PartialEq)]
enum Panel {
    /// Registry key tree view - the main navigation interface
    Tree,
    /// Search interface for finding keys and values
    Search,
    /// Saved bookmarks list
    Bookmarks,
    /// Changes waiting to be synced to the registry
    PendingChanges,
}

/// How changes are handled - manual staging or auto-commit.
///
/// # Manual Mode (Default)
/// Changes are written to SQLite but NOT to the registry.
/// User must explicitly "Push to Registry" to apply changes.
/// This allows preview and batch changes.
///
/// # AutoSync Mode
/// Changes are immediately pushed to the registry after each edit.
/// More like traditional registry editors.
#[derive(PartialEq, Clone)]
enum SyncMode {
    /// Changes are staged in SQLite, must be explicitly pushed
    Manual,
    /// Changes are automatically pushed to registry immediately  
    AutoSync,
}

/// Tracks which dialog (modal popup) is currently open.
///
/// # Rust Concept: Enums with Associated Data
///
/// Each variant can hold different data relevant to that dialog.
/// This is called "algebraic data types" or "tagged unions" in other languages.
///
/// ```rust
/// // The enum variant tells us WHAT dialog is open
/// // The associated data tells us the CURRENT STATE of that dialog
/// EditDialog::NewKey(String::from("MyNewKey"))
/// //           ↑ variant        ↑ current name being typed
/// ```
///
/// # Why Clone?
///
/// We clone this enum in `show_dialogs()` because we need to:
/// 1. Match on it (borrows self)
/// 2. Potentially modify self.edit_dialog (needs mut borrow)
///
/// Cloning lets us work around Rust's borrow rules.
#[derive(PartialEq, Clone)]
enum EditDialog {
    /// No dialog open - normal UI state
    None,
    /// Creating new key - stores the name being typed
    NewKey(String),
    /// Renaming a value - (old_name, new_name being typed)
    RenameValue(String, String),
    /// Editing string value - (name, data, is_expandable_string)
    EditStringValue(String, String, bool),
    /// Editing DWORD - (name, value as string for editing)
    EditDwordValue(String, String),
    /// Editing QWORD - (name, value as string for editing)  
    EditQwordValue(String, String),
    /// Editing binary - (name, hex string representation)
    EditBinaryValue(String, String),
    /// Editing multi-string - (name, lines joined by newlines)
    EditMultiStringValue(String, String),
    /// Creating new value - (name, type_index into types array)
    NewValue(String, usize),
    /// Adding bookmark - (display_name, notes, color_index)
    AddBookmark(String, String, usize),
    /// Editing bookmark - (list_index, name, notes, color_index)
    EditBookmark(usize, String, String, usize),
    /// Confirming deletion - (name, is_key vs is_value)
    ConfirmDelete(String, bool),
    /// Showing sync conflicts from push operation
    SyncConflicts(Vec<SyncConflict>),
    /// Confirming discard of all pending changes
    ConfirmDiscardChanges,
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// MAIN APPLICATION STATE
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// The main application struct holding all UI state.
///
/// # Rust Concept: Struct Organization
///
/// This struct groups related fields with comments. In a larger project,
/// you might split this into multiple structs, but for egui apps it's
/// often simpler to keep state in one place.
///
/// # Immediate Mode GUI State
///
/// In immediate mode GUIs, we need to store:
/// - **Navigation state**: Where the user is (selected key, path)
/// - **UI state**: Which panel is active, dialog open, etc.
/// - **Cached data**: Values to display, search results, etc.
/// - **Backend handles**: Database connection, sync state, etc.
///
/// # Memory Note
///
/// This struct is stored on the heap by eframe and persists for the
/// lifetime of the application.
pub struct RegistryEditorApp {
    // ─────────────────────────────────────────────────────────────────────
    // Navigation State - Where is the user in the registry tree?
    // ─────────────────────────────────────────────────────────────────────
    
    /// Currently selected registry root (HKEY_CURRENT_USER, etc.)
    /// `None` means no key is selected yet.
    selected_root: Option<RootKey>,
    
    /// Path within the selected root (e.g., "Software\\Microsoft")
    /// Empty string means the root itself is selected.
    selected_path: String,
    
    /// Set of full paths that are expanded in the tree view.
    /// Used to remember expansion state when navigating.
    /// Example: {"HKEY_CURRENT_USER", "HKEY_CURRENT_USER\\Software"}
    expanded_keys: std::collections::HashSet<String>,

    // ─────────────────────────────────────────────────────────────────────
    // Values Display - What's shown in the right panel?
    // ─────────────────────────────────────────────────────────────────────
    
    /// Values in the currently selected key (cached for display).
    /// Refreshed when navigation changes via `refresh_values()`.
    values: Vec<RegistryValue>,
    
    /// Index of selected value in the values list, if any.
    selected_value: Option<usize>,

    // ─────────────────────────────────────────────────────────────────────
    // Search State - Managing search operations
    // ─────────────────────────────────────────────────────────────────────
    
    /// Current search configuration (query, filters, etc.)
    search_options: SearchOptions,
    
    /// Shared search state (results, progress, cancellation)
    search_state: SearchState,
    
    /// Snapshot of search results for stable display during search.
    /// Updated periodically during search to show progress.
    search_results_snapshot: Vec<SearchResult>,

    // ─────────────────────────────────────────────────────────────────────
    // Backend - Database and sync
    // ─────────────────────────────────────────────────────────────────────
    
    /// SQLite-first store with registry sync capabilities.
    /// This is the single source of truth for all data operations.
    store: SyncStore,

    /// How changes are handled (manual staging vs auto-sync)
    sync_mode: SyncMode,

    // ─────────────────────────────────────────────────────────────────────
    // UI State - Dialogs, panels, messages
    // ─────────────────────────────────────────────────────────────────────
    
    /// Which tab/panel is active in the left sidebar
    active_panel: Panel,
    
    /// Contents of the path bar (editable address bar)
    path_bar: String,
    
    /// Status message shown in the bottom bar
    status_message: String,
    
    /// Currently open dialog (if any)
    edit_dialog: EditDialog,
    
    /// Error message to display (shown in a modal)
    error_message: Option<String>,
    
    /// Whether search options panel is expanded
    show_search_options: bool,
    
    /// Whether sync settings panel is expanded
    show_sync_settings: bool,
    
    /// Track if we were syncing last frame (to detect completion)
    was_syncing: bool,
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// APPLICATION INITIALIZATION
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

impl RegistryEditorApp {
    /// Create a new application instance.
    ///
    /// # What This Does
    ///
    /// 1. Creates the SQLite-backed SyncStore
    /// 2. Starts background registry synchronization
    /// 3. Initializes all UI state to defaults
    ///
    /// # The `_cc` Parameter
    ///
    /// `CreationContext` provides access to egui's context during creation.
    /// We prefix with `_` because we don't use it, but the trait requires it.
    pub fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        // Create the store - this opens/creates the SQLite database
        let store = SyncStore::new();
        
        // Start background sync to populate cache from registry
        store.start_background_pull();

        Self {
            // Navigation - start with nothing selected
            selected_root: None,
            selected_path: String::new(),
            expanded_keys: std::collections::HashSet::new(),
            
            // Values - empty until a key is selected
            values: Vec::new(),
            selected_value: None,
            
            // Search - default options, new state
            search_options: SearchOptions::default(),
            search_state: SearchState::new(),
            search_results_snapshot: Vec::new(),
            
            // Backend
            store,
            sync_mode: SyncMode::Manual,
            
            // UI state
            active_panel: Panel::Tree,
            path_bar: String::new(),
            status_message: String::new(),
            edit_dialog: EditDialog::None,
            error_message: None,
            show_search_options: false,
            show_sync_settings: false,
            was_syncing: false,
        }
    }

    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    // NAVIGATION HELPERS
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

    /// Get the full path to the currently selected key.
    ///
    /// # Returns
    ///
    /// - "HKEY_CURRENT_USER\\Software\\MyApp" for a subkey
    /// - "HKEY_CURRENT_USER" for a root key
    /// - "" if nothing is selected
    fn full_path(&self) -> String {
        if let Some(ref root) = self.selected_root {
            if self.selected_path.is_empty() {
                root.to_string()
            } else {
                format!("{}\\{}", root, self.selected_path)
            }
        } else {
            String::new()
        }
    }

    /// Navigate to a specific registry path.
    ///
    /// # What This Does
    ///
    /// 1. Parses the path to extract root and subpath
    /// 2. Verifies the path exists in our SQLite cache
    /// 3. If not found, treats input as a search query
    /// 4. Expands all parent nodes in the tree
    /// 5. Updates selection and refreshes values
    ///
    /// # Arguments
    ///
    /// * `full_path` - Full path like "HKEY_CURRENT_USER\\Software\\MyApp"
    fn navigate_to_path(&mut self, full_path: &str) {
        // Parse "HKEY_XXX\path\to\key"
        let parts: Vec<&str> = full_path.splitn(2, '\\').collect();
        let root_name = parts[0];
        let sub_path = if parts.len() > 1 { parts[1] } else { "" };

        if let Some(root) = RootKey::from_name(root_name) {
            // Verify the key actually exists in our SQLite store
            if !sub_path.is_empty() {
                if !self.store.key_exists(&root, sub_path) {
                    // Key doesn't exist — treat input as a search
                    self.run_path_bar_search(full_path);
                    return;
                }
            }

            // Expand all parent keys
            let mut cumulative = root.to_string();
            self.expanded_keys.insert(cumulative.clone());
            if !sub_path.is_empty() {
                for segment in sub_path.split('\\') {
                    cumulative = format!("{}\\{}", cumulative, segment);
                    self.expanded_keys.insert(cumulative.clone());
                }
            }

            self.selected_root = Some(root.clone());
            self.selected_path = sub_path.to_string();
            self.path_bar = full_path.to_string();
            self.refresh_values();
        } else {
            // Doesn't start with a valid root — treat as search
            self.run_path_bar_search(full_path);
        }
    }

    fn run_path_bar_search(&mut self, query: &str) {
        self.search_options.query = query.to_string();
        self.active_panel = Panel::Search;
        // Use SQLite-based search from the sync store
        rust_hive::search::start_search(
            self.search_options.clone(),
            self.search_state.clone(),
            None, // No longer using the old index
        );
        self.status_message = format!("Searching for \"{}\"...", query);
    }

    fn refresh_values(&mut self) {
        if let Some(ref root) = self.selected_root {
            match self.store.get_values(root, &self.selected_path) {
                Ok(vals) => self.values = vals,
                Err(e) => {
                    self.values.clear();
                    self.status_message = format!("Error: {}", e);
                }
            }
            self.selected_value = None;
        }
    }

    fn get_subkeys_cached(&mut self, root: &RootKey, path: &str) -> Vec<String> {
        self.store.get_subkeys(root, path)
    }

    /// If auto-sync is enabled, immediately push changes to the registry
    fn maybe_auto_sync(&mut self) {
        if self.sync_mode == SyncMode::AutoSync {
            self.store.push_to_registry_async();
        }
    }

    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    // MENU BAR
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

    /// Render the top menu bar.
    ///
    /// # Menus
    ///
    /// - **File**: Import/export .reg files, exit
    /// - **Edit**: New key/value, delete, copy path
    /// - **Bookmarks**: Add/remove bookmarks, quick-nav
    /// - **View**: Refresh
    /// - **Sync**: Push/pull operations, sync settings
    fn show_menu_bar(&mut self, ui: &mut egui::Ui) {
        egui::menu::bar(ui, |ui| {
            ui.menu_button("File", |ui| {
                if ui.button("Export .reg file...").clicked() {
                    self.export_reg_file();
                    ui.close_menu();
                }
                if ui.button("Import .reg file...").clicked() {
                    self.import_reg_file();
                    ui.close_menu();
                }
                ui.separator();
                if ui.button("Exit").clicked() {
                    std::process::exit(0);
                }
            });

            ui.menu_button("Edit", |ui| {
                let has_selection = self.selected_root.is_some();
                if ui
                    .add_enabled(has_selection, egui::Button::new("New Key..."))
                    .clicked()
                {
                    self.edit_dialog = EditDialog::NewKey(String::from("New Key #1"));
                    ui.close_menu();
                }
                if ui
                    .add_enabled(has_selection, egui::Button::new("New Value..."))
                    .clicked()
                {
                    self.edit_dialog = EditDialog::NewValue(String::new(), 0);
                    ui.close_menu();
                }
                ui.separator();
                if ui
                    .add_enabled(has_selection, egui::Button::new("Delete Key"))
                    .clicked()
                {
                    if let Some(ref _root) = self.selected_root {
                        if !self.selected_path.is_empty() {
                            let name = self
                                .selected_path
                                .rsplit('\\')
                                .next()
                                .unwrap_or("")
                                .to_string();
                            self.edit_dialog = EditDialog::ConfirmDelete(name, true);
                        }
                    }
                    ui.close_menu();
                }
                ui.separator();
                if ui.button("Copy Key Path").clicked() {
                    let path = self.full_path();
                    if let Ok(mut clipboard) = arboard::Clipboard::new() {
                        clipboard.set_text(&path).ok();
                    }
                    self.status_message = "Path copied to clipboard".to_string();
                    ui.close_menu();
                }
            });

            ui.menu_button("Bookmarks", |ui| {
                let path = self.full_path();
                if !path.is_empty() {
                    if self.store.is_bookmarked(&path) {
                        if ui.button("Remove Bookmark").clicked() {
                            self.store.remove_bookmark(&path);
                            ui.close_menu();
                        }
                    } else {
                        if ui.button("Add Bookmark...").clicked() {
                            let name = path.rsplit('\\').next().unwrap_or(&path).to_string();
                            self.edit_dialog = EditDialog::AddBookmark(name, String::new(), 0);
                            ui.close_menu();
                        }
                    }
                }
                ui.separator();
                let bookmarks_clone = self.store.get_bookmarks();
                for bm in bookmarks_clone.iter() {
                    let label = if let Some(ref color) = bm.color {
                        let (r, g, b) = color.to_rgb();
                        egui::RichText::new(&bm.name).color(egui::Color32::from_rgb(r, g, b))
                    } else {
                        egui::RichText::new(&bm.name)
                    };
                    if ui.button(label).clicked() {
                        let path = bm.path.clone();
                        self.navigate_to_path(&path);
                        self.active_panel = Panel::Tree;
                        ui.close_menu();
                    }
                }
                if bookmarks_clone.is_empty() {
                    ui.label("No bookmarks yet");
                }
            });

            ui.menu_button("View", |ui| {
                if ui.button("Refresh (F5)").clicked() {
                    if let Some(ref root) = self.selected_root {
                        self.store.refresh_key(root, &self.selected_path);
                    }
                    self.refresh_values();
                    ui.close_menu();
                }
            });

            // Sync menu
            ui.menu_button("Sync", |ui| {
                let pending = self.store.pending_change_count();
                let is_syncing = self.store.is_syncing.load(Ordering::Relaxed);

                if pending > 0 {
                    ui.label(egui::RichText::new(format!("{} pending changes", pending))
                        .color(egui::Color32::from_rgb(255, 200, 100)));
                    ui.separator();
                }

                if !is_syncing {
                    if ui.button("⬆ Push to Registry").clicked() {
                        self.store.push_to_registry_async();
                        self.status_message = "Pushing changes to registry...".to_string();
                        ui.close_menu();
                    }

                    if ui.button("⬇ Pull from Registry").clicked() {
                        self.store.pull_from_registry_async();
                        self.status_message = "Pulling latest from registry...".to_string();
                        ui.close_menu();
                    }
                } else {
                    ui.spinner();
                    ui.label("Syncing...");
                }

                ui.separator();

                if pending > 0 && ui.button("View Pending Changes").clicked() {
                    self.active_panel = Panel::PendingChanges;
                    ui.close_menu();
                }

                if pending > 0 && ui.button("Discard All Changes").clicked() {
                    self.edit_dialog = EditDialog::ConfirmDiscardChanges;
                    ui.close_menu();
                }

                ui.separator();
                ui.label("Sync Mode:");
                if ui.radio(self.sync_mode == SyncMode::Manual, "Manual").clicked() {
                    self.sync_mode = SyncMode::Manual;
                }
                if ui.radio(self.sync_mode == SyncMode::AutoSync, "Auto-sync").clicked() {
                    self.sync_mode = SyncMode::AutoSync;
                }
            });
        });
    }

    fn show_path_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label("Path:");
            let response = ui.add(
                egui::TextEdit::singleline(&mut self.path_bar)
                    .desired_width(ui.available_width() - 80.0)
                    .hint_text("Path or search..."),
            );
            if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                let path = self.path_bar.clone();
                self.navigate_to_path(&path);
            }
            if ui.button("Go").clicked() {
                let path = self.path_bar.clone();
                self.navigate_to_path(&path);
            }
        });
    }

    fn show_left_panel(&mut self, ui: &mut egui::Ui) {
        // Tab buttons
        ui.horizontal(|ui| {
            if ui
                .selectable_label(self.active_panel == Panel::Tree, "Registry")
                .clicked()
            {
                self.active_panel = Panel::Tree;
            }
            if ui
                .selectable_label(self.active_panel == Panel::Search, "Search")
                .clicked()
            {
                self.active_panel = Panel::Search;
            }
            if ui
                .selectable_label(self.active_panel == Panel::Bookmarks, "Bookmarks")
                .clicked()
            {
                self.active_panel = Panel::Bookmarks;
            }
            // Show pending changes indicator
            let pending = self.store.pending_change_count();
            if pending > 0 {
                let label = format!("Changes ({})", pending);
                if ui
                    .selectable_label(self.active_panel == Panel::PendingChanges, label)
                    .clicked()
                {
                    self.active_panel = Panel::PendingChanges;
                }
            }
        });

        ui.separator();

        match self.active_panel {
            Panel::Tree => self.show_tree_panel(ui),
            Panel::Search => self.show_search_panel(ui),
            Panel::Bookmarks => self.show_bookmarks_panel(ui),
            Panel::PendingChanges => self.show_pending_changes_panel(ui),
        }
    }

    fn show_tree_panel(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::both().show(ui, |ui| {
            // Collect tree items to render
            let mut nodes_to_render: Vec<(RootKey, String)> = Vec::new();
            for root in RootKey::all() {
                nodes_to_render.push((root.clone(), String::new()));
            }
            self.render_tree_nodes(ui, &nodes_to_render);
        });
    }

    fn render_tree_nodes(&mut self, ui: &mut egui::Ui, nodes: &[(RootKey, String)]) {
        for (root, path) in nodes {
            let full_key = if path.is_empty() {
                root.to_string()
            } else {
                format!("{}\\{}", root, path)
            };

            let display_name = if path.is_empty() {
                root.to_string()
            } else {
                path.rsplit('\\').next().unwrap_or(path).to_string()
            };

            let is_selected = self.selected_root.as_ref() == Some(root)
                && self.selected_path == *path;

            let subkeys = self.get_subkeys_cached(root, path);
            // Root keys (empty path) always have children, show as expandable
            let has_children = !subkeys.is_empty() || path.is_empty();

            if has_children {
                let default_open = self.expanded_keys.contains(&full_key);
                let id = ui.make_persistent_id(&full_key);
                let resp = egui::CollapsingHeader::new(
                    if is_selected {
                        egui::RichText::new(&display_name).strong()
                    } else {
                        egui::RichText::new(&display_name)
                    },
                )
                .id_salt(id)
                .default_open(default_open)
                .show(ui, |ui| {
                    let child_nodes: Vec<(RootKey, String)> = subkeys
                        .iter()
                        .map(|subkey| {
                            let child_path = if path.is_empty() {
                                subkey.clone()
                            } else {
                                format!("{}\\{}", path, subkey)
                            };
                            (root.clone(), child_path)
                        })
                        .collect();
                    self.render_tree_nodes(ui, &child_nodes);
                });

                // Track expansion state
                if resp.openness > 0.0 {
                    self.expanded_keys.insert(full_key.clone());
                } else {
                    self.expanded_keys.remove(&full_key);
                }

                // Handle click on header
                if resp.header_response.clicked() {
                    self.selected_root = Some(root.clone());
                    self.selected_path = path.to_string();
                    self.path_bar = full_key.clone();
                    self.refresh_values();
                }

                // Context menu
                self.tree_node_context_menu(&resp.header_response, root, path, &full_key);
            } else {
                // Leaf node
                let resp = ui.selectable_label(is_selected, &display_name);
                if resp.clicked() {
                    self.selected_root = Some(root.clone());
                    self.selected_path = path.to_string();
                    self.path_bar = full_key.clone();
                    self.refresh_values();
                }

                // Context menu
                self.tree_node_context_menu(&resp, root, path, &full_key);
            }
        }
    }

    fn tree_node_context_menu(
        &mut self,
        response: &egui::Response,
        root: &RootKey,
        path: &str,
        full_key: &str,
    ) {
        response.context_menu(|ui| {
            if ui.button("Bookmark").clicked() {
                let name = if path.is_empty() {
                    root.to_string()
                } else {
                    path.rsplit('\\').next().unwrap_or(path).to_string()
                };
                if self.store.is_bookmarked(full_key) {
                    self.store.remove_bookmark(full_key);
                    self.status_message = format!("Removed bookmark: {}", name);
                } else {
                    self.edit_dialog =
                        EditDialog::AddBookmark(name, String::new(), 0);
                    // Navigate to this key so the bookmark gets the right path
                    self.selected_root = Some(root.clone());
                    self.selected_path = path.to_string();
                    self.path_bar = full_key.to_string();
                    self.refresh_values();
                }
                ui.close_menu();
            }

            ui.separator();

            if ui.button("New Key...").clicked() {
                self.selected_root = Some(root.clone());
                self.selected_path = path.to_string();
                self.path_bar = full_key.to_string();
                self.refresh_values();
                self.edit_dialog = EditDialog::NewKey(String::from("New Key #1"));
                ui.close_menu();
            }

            if ui.button("New Value...").clicked() {
                self.selected_root = Some(root.clone());
                self.selected_path = path.to_string();
                self.path_bar = full_key.to_string();
                self.refresh_values();
                self.edit_dialog = EditDialog::NewValue(String::new(), 0);
                ui.close_menu();
            }

            if !path.is_empty() {
                if ui.button("Delete Key").clicked() {
                    self.selected_root = Some(root.clone());
                    self.selected_path = path.to_string();
                    self.path_bar = full_key.to_string();
                    let name = path.rsplit('\\').next().unwrap_or(path).to_string();
                    self.edit_dialog = EditDialog::ConfirmDelete(name, true);
                    ui.close_menu();
                }
            }

            ui.separator();

            if ui.button("Copy Path").clicked() {
                if let Ok(mut clipboard) = arboard::Clipboard::new() {
                    clipboard.set_text(full_key).ok();
                }
                self.status_message = "Path copied to clipboard".to_string();
                ui.close_menu();
            }
        });
    }

    fn show_search_panel(&mut self, ui: &mut egui::Ui) {
        // Search input
        ui.horizontal(|ui| {
            let response = ui.add(
                egui::TextEdit::singleline(&mut self.search_options.query)
                    .desired_width(ui.available_width() - 120.0)
                    .hint_text("Search registry..."),
            );

            let is_searching = self.search_state.is_searching.load(Ordering::Relaxed);

            if !is_searching {
                if ui.button("Search").clicked()
                    || (response.lost_focus()
                        && ui.input(|i| i.key_pressed(egui::Key::Enter)))
                {
                    if !self.search_options.query.is_empty() {
                        // Use the sync store's search or fall back to live search
                        rust_hive::search::start_search(
                            self.search_options.clone(),
                            self.search_state.clone(),
                            None,
                        );
                    }
                }
            } else {
                if ui.button("Cancel").clicked() {
                    self.search_state.cancel.store(true, Ordering::SeqCst);
                }
            }
        });

        // Search options toggle
        if ui
            .selectable_label(self.show_search_options, "Options")
            .clicked()
        {
            self.show_search_options = !self.show_search_options;
        }

        if self.show_search_options {
            ui.group(|ui| {
                ui.checkbox(&mut self.search_options.use_regex, "Regex");
                ui.checkbox(&mut self.search_options.case_sensitive, "Case sensitive");
                ui.separator();
                ui.label("Search in:");
                ui.checkbox(&mut self.search_options.search_keys, "Key names");
                ui.checkbox(&mut self.search_options.search_value_names, "Value names");
                ui.checkbox(&mut self.search_options.search_value_data, "Value data");
                ui.separator();
                ui.label("Search roots:");
                let all_roots = RootKey::all().to_vec();
                for root in &all_roots {
                    let mut checked = self.search_options.roots_to_search.contains(root);
                    if ui.checkbox(&mut checked, root.to_string()).changed() {
                        if checked {
                            if !self.search_options.roots_to_search.contains(root) {
                                self.search_options.roots_to_search.push(root.clone());
                            }
                        } else {
                            self.search_options
                                .roots_to_search
                                .retain(|r| r != root);
                        }
                    }
                }
                ui.separator();
                ui.horizontal(|ui| {
                    ui.label("Max results:");
                    let mut max_str = self.search_options.max_results.to_string();
                    if ui
                        .add(egui::TextEdit::singleline(&mut max_str).desired_width(80.0))
                        .changed()
                    {
                        if let Ok(n) = max_str.parse::<usize>() {
                            self.search_options.max_results = n;
                        }
                    }
                });

                ui.horizontal(|ui| {
                    ui.label("Max depth:");
                    let mut depth_str = self
                        .search_options
                        .max_depth
                        .map(|d| d.to_string())
                        .unwrap_or_default();
                    if ui
                        .add(
                            egui::TextEdit::singleline(&mut depth_str)
                                .desired_width(80.0)
                                .hint_text("unlimited"),
                        )
                        .changed()
                    {
                        self.search_options.max_depth = depth_str.parse::<usize>().ok();
                    }
                });

                ui.horizontal(|ui| {
                    ui.label("Value type:");
                    let types = [
                        "",
                        "REG_SZ",
                        "REG_EXPAND_SZ",
                        "REG_MULTI_SZ",
                        "REG_DWORD",
                        "REG_QWORD",
                        "REG_BINARY",
                    ];
                    let current = self
                        .search_options
                        .value_type_filter
                        .clone()
                        .unwrap_or_default();
                    egui::ComboBox::from_id_salt("value_type_filter")
                        .selected_text(if current.is_empty() { "Any" } else { &current })
                        .show_ui(ui, |ui| {
                            for &ty in &types {
                                let label = if ty.is_empty() { "Any" } else { ty };
                                if ui.selectable_label(current == ty, label).clicked() {
                                    self.search_options.value_type_filter = if ty.is_empty() {
                                        None
                                    } else {
                                        Some(ty.to_string())
                                    };
                                }
                            }
                        });
                });
            });
        }

        // Sync status (replaces old index status)
        ui.separator();
        {
            let is_syncing = self.store.is_syncing.load(Ordering::Relaxed);
            let pending = self.store.pending_change_count();

            ui.horizontal(|ui| {
                if is_syncing {
                    ui.spinner();
                    let progress = self.store.sync_progress.load(Ordering::Relaxed);
                    let total = self.store.sync_total.load(Ordering::Relaxed);
                    ui.label(
                        egui::RichText::new(format!("Syncing... {}/{}", progress, total))
                            .small()
                            .color(egui::Color32::from_rgb(255, 200, 100)),
                    );
                    ui.ctx().request_repaint();
                } else if pending > 0 {
                    ui.label(
                        egui::RichText::new(format!("⚠ {} pending changes", pending))
                            .small()
                            .color(egui::Color32::from_rgb(255, 200, 100)),
                    );
                    if ui.small_button("Push").clicked() {
                        self.store.push_to_registry_async();
                        self.status_message = "Pushing changes...".to_string();
                    }
                } else {
                    ui.label(
                        egui::RichText::new("✓ Synced with registry")
                            .small()
                            .color(egui::Color32::from_rgb(100, 200, 100)),
                    );
                }

                if ui.small_button("Pull").clicked() {
                    self.store.pull_from_registry_async();
                    self.status_message = "Pulling...".to_string();
                }
            });

            // Sync settings (inside search options)
            if self.show_search_options {
                ui.collapsing("Sync Settings", |ui| {
                    let mut enabled = self.store.auto_pull_enabled.load(Ordering::Relaxed);
                    if ui.checkbox(&mut enabled, "Auto-pull enabled").changed() {
                        self.store.auto_pull_enabled.store(enabled, Ordering::SeqCst);
                    }

                    ui.horizontal(|ui| {
                        ui.label("Pull interval:");
                        let mut secs = self
                            .store
                            .auto_pull_interval_secs
                            .lock()
                            .unwrap()
                            .to_string();
                        if ui
                            .add(
                                egui::TextEdit::singleline(&mut secs)
                                    .desired_width(60.0),
                            )
                            .changed()
                        {
                            if let Ok(n) = secs.parse::<u64>() {
                                *self.store.auto_pull_interval_secs.lock().unwrap() = n;
                            }
                        }
                        ui.label("sec");
                    });

                    ui.horizontal(|ui| {
                        ui.label("Max depth:");
                        let depth = self.store.pull_max_depth.lock().unwrap();
                        let mut depth_str = depth
                            .map(|d| d.to_string())
                            .unwrap_or_default();
                        drop(depth);
                        if ui
                            .add(
                                egui::TextEdit::singleline(&mut depth_str)
                                    .desired_width(60.0)
                                    .hint_text("unlimited"),
                            )
                            .changed()
                        {
                            *self.store.pull_max_depth.lock().unwrap() =
                                depth_str.parse::<usize>().ok();
                        }
                    });
                });
            }
        }

        ui.separator();

        // Search status
        let is_searching = self.search_state.is_searching.load(Ordering::Relaxed);
        if is_searching {
            let scanned = self.search_state.keys_scanned.load(Ordering::Relaxed);
            let count = self.search_state.results.lock().unwrap().len();
            let current = self.search_state.current_path.lock().unwrap().clone();
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(format!(
                    "Searching... {} keys scanned, {} results",
                    scanned, count
                ));
            });
            if !current.is_empty() {
                ui.label(
                    egui::RichText::new(&current)
                        .small()
                        .color(egui::Color32::GRAY),
                );
            }
            ui.ctx().request_repaint();

            // Update snapshot periodically
            self.search_results_snapshot =
                self.search_state.results.lock().unwrap().clone();
        } else if !self.search_results_snapshot.is_empty()
            || !self.search_state.results.lock().unwrap().is_empty()
        {
            // Final snapshot
            let results = self.search_state.results.lock().unwrap();
            if results.len() != self.search_results_snapshot.len() {
                self.search_results_snapshot = results.clone();
            }
            drop(results);
            ui.label(format!("{} results", self.search_results_snapshot.len()));
        }

        // Results list
        egui::ScrollArea::both().show(ui, |ui| {
            let mut navigate_to: Option<String> = None;

            for result in &self.search_results_snapshot {
                let icon = match result.match_type {
                    MatchType::KeyName => "K",
                    MatchType::ValueName => "N",
                    MatchType::ValueData => "D",
                };

                let text = if let Some(ref vname) = result.value_name {
                    format!(
                        "[{}] {}\\{} = {}",
                        icon,
                        result.full_path(),
                        vname,
                        result.value_data.as_deref().unwrap_or("")
                    )
                } else {
                    format!("[{}] {}", icon, result.full_path())
                };

                let resp = ui.add(
                    egui::Label::new(
                        egui::RichText::new(&text).small().color(
                            match result.match_type {
                                MatchType::KeyName => egui::Color32::from_rgb(100, 180, 255),
                                MatchType::ValueName => egui::Color32::from_rgb(100, 255, 100),
                                MatchType::ValueData => egui::Color32::from_rgb(255, 200, 100),
                            },
                        ),
                    )
                    .sense(egui::Sense::click()),
                );

                if resp.clicked() {
                    navigate_to = Some(result.full_path());
                }

                resp.on_hover_text(format!(
                    "Match: {}\nPath: {}",
                    result.match_type,
                    result.full_path()
                ));
            }

            if let Some(path) = navigate_to {
                self.navigate_to_path(&path);
                self.active_panel = Panel::Tree;
            }
        });
    }

    fn show_bookmarks_panel(&mut self, ui: &mut egui::Ui) {
        if ui.button("Add current key").clicked() {
            let path = self.full_path();
            if !path.is_empty() {
                let name = path.rsplit('\\').next().unwrap_or(&path).to_string();
                self.edit_dialog = EditDialog::AddBookmark(name, String::new(), 0);
            }
        }

        ui.separator();

        egui::ScrollArea::both().show(ui, |ui| {
            let mut navigate_to: Option<String> = None;
            let mut action: Option<BookmarkAction> = None;

            let bookmarks_clone = self.store.get_bookmarks();
            let bm_count = bookmarks_clone.len();

            for (i, bm) in bookmarks_clone.iter().enumerate() {
                ui.horizontal(|ui| {
                    // Color indicator
                    if let Some(ref color) = bm.color {
                        let (r, g, b) = color.to_rgb();
                        let c = egui::Color32::from_rgb(r, g, b);
                        let (rect, _) = ui.allocate_exact_size(
                            egui::vec2(4.0, 16.0),
                            egui::Sense::hover(),
                        );
                        ui.painter().rect_filled(rect, 2.0, c);
                    }

                    // Bookmark name (clickable)
                    let label = egui::RichText::new(&bm.name);
                    if ui
                        .add(egui::Label::new(label).sense(egui::Sense::click()))
                        .clicked()
                    {
                        navigate_to = Some(bm.path.clone());
                    }

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button("x").clicked() {
                            action = Some(BookmarkAction::Remove(i));
                        }
                        if ui.small_button("E").clicked() {
                            let color_idx = bm
                                .color
                                .as_ref()
                                .and_then(|c| {
                                    BookmarkColor::all()
                                        .iter()
                                        .position(|x| x == c)
                                })
                                .map(|i| i + 1) // +1 because 0 = None
                                .unwrap_or(0);
                            action = Some(BookmarkAction::Edit(
                                i,
                                bm.name.clone(),
                                bm.notes.clone(),
                                color_idx,
                            ));
                        }
                        if i + 1 < bm_count && ui.small_button("v").clicked() {
                            action = Some(BookmarkAction::MoveDown(i));
                        }
                        if i > 0 && ui.small_button("^").clicked() {
                            action = Some(BookmarkAction::MoveUp(i));
                        }
                    });
                });

                // Show path and notes as secondary info
                ui.label(
                    egui::RichText::new(&bm.path)
                        .small()
                        .color(egui::Color32::GRAY),
                );
                if !bm.notes.is_empty() {
                    ui.label(
                        egui::RichText::new(&bm.notes)
                            .small()
                            .italics()
                            .color(egui::Color32::from_rgb(180, 180, 180)),
                    );
                }
                ui.separator();
            }

            if bookmarks_clone.is_empty() {
                ui.label("No bookmarks. Navigate to a key and click 'Add current key'.");
            }

            // Apply actions
            match action {
                Some(BookmarkAction::Remove(i)) => {
                    if i < bookmarks_clone.len() {
                        let path = bookmarks_clone[i].path.clone();
                        self.store.remove_bookmark(&path);
                    }
                }
                Some(BookmarkAction::MoveUp(i)) => {
                    if i < bookmarks_clone.len() {
                        let path = bookmarks_clone[i].path.clone();
                        self.store.move_bookmark(&path, -1);
                    }
                }
                Some(BookmarkAction::MoveDown(i)) => {
                    if i < bookmarks_clone.len() {
                        let path = bookmarks_clone[i].path.clone();
                        self.store.move_bookmark(&path, 1);
                    }
                }
                Some(BookmarkAction::Edit(i, name, notes, color_idx)) => {
                    self.edit_dialog = EditDialog::EditBookmark(i, name, notes, color_idx);
                }
                None => {}
            }

            if let Some(path) = navigate_to {
                self.navigate_to_path(&path);
                self.active_panel = Panel::Tree;
            }
        });
    }

    fn show_pending_changes_panel(&mut self, ui: &mut egui::Ui) {
        let is_syncing = self.store.is_syncing.load(Ordering::Relaxed);
        
        ui.horizontal(|ui| {
            if !is_syncing {
                if ui.button("⬆ Push All to Registry").clicked() {
                    self.store.push_to_registry_async();
                    self.status_message = "Pushing changes to registry...".to_string();
                }
                if ui.button("Discard All").clicked() {
                    self.edit_dialog = EditDialog::ConfirmDiscardChanges;
                }
            } else {
                ui.spinner();
                let progress = self.store.sync_progress.load(Ordering::Relaxed);
                let total = self.store.sync_total.load(Ordering::Relaxed);
                ui.label(format!("Syncing... {}/{}", progress, total));
            }
        });

        ui.separator();

        let changes = self.store.get_pending_changes();
        
        if changes.is_empty() {
            ui.label("No pending changes. All edits are saved to the registry.");
            return;
        }

        ui.label(format!("{} pending changes:", changes.len()));
        ui.separator();

        egui::ScrollArea::both().show(ui, |ui| {
            let mut discard_id: Option<i64> = None;

            for (id, change) in &changes {
                ui.horizontal(|ui| {
                    // Icon based on change type
                    let icon = match change {
                        PendingChange::CreateKey { .. } => "➕",
                        PendingChange::DeleteKey { .. } => "❌",
                        PendingChange::SetValue { .. } => "✏️",
                        PendingChange::DeleteValue { .. } => "🗑️",
                        PendingChange::RenameValue { .. } => "📝",
                    };
                    ui.label(icon);
                    
                    // Description
                    ui.label(change.description());

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button("Discard").clicked() {
                            discard_id = Some(*id);
                        }
                        if ui.small_button("Go to").clicked() {
                            self.navigate_to_path(&change.full_path());
                            self.active_panel = Panel::Tree;
                        }
                    });
                });
                ui.separator();
            }

            if let Some(id) = discard_id {
                self.store.discard_pending_change(id);
            }
        });
    }

    fn show_values_panel(&mut self, ui: &mut egui::Ui) {
        // Toolbar
        ui.horizontal(|ui| {
            if ui.button("New Value").clicked() && self.selected_root.is_some() {
                self.edit_dialog = EditDialog::NewValue(String::new(), 0);
            }
            if ui.button("Refresh").clicked() {
                if let Some(ref root) = self.selected_root {
                    self.store.refresh_key(root, &self.selected_path);
                }
                self.refresh_values();
            }

            let path = self.full_path();
            if !path.is_empty() {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let is_bookmarked = self.store.is_bookmarked(&path);
                    let bm_label = if is_bookmarked {
                        "Unbookmark"
                    } else {
                        "Bookmark"
                    };
                    if ui.button(bm_label).clicked() {
                        if is_bookmarked {
                            self.store.remove_bookmark(&path);
                        } else {
                            let name = path.rsplit('\\').next().unwrap_or(&path).to_string();
                            self.edit_dialog =
                                EditDialog::AddBookmark(name, String::new(), 0);
                        }
                    }

                    // Show sync status indicator
                    let pending = self.store.pending_change_count();
                    if pending > 0 {
                        ui.label(
                            egui::RichText::new(format!("\u{26a0} {} unsaved", pending))
                                .color(egui::Color32::from_rgb(255, 200, 100)),
                        );
                    }
                });
            }
        });

        ui.separator();

        // Column headers
        egui::Grid::new("values_header")
            .num_columns(3)
            .striped(false)
            .min_col_width(100.0)
            .show(ui, |ui| {
                ui.strong("Name");
                ui.strong("Type");
                ui.strong("Data");
                ui.end_row();
            });

        ui.separator();

        // Values list
        egui::ScrollArea::both().show(ui, |ui| {
            let values_clone = self.values.clone();
            let mut edit_value: Option<usize> = None;
            let mut delete_value: Option<usize> = None;

            egui::Grid::new("values_grid")
                .num_columns(3)
                .striped(true)
                .min_col_width(100.0)
                .show(ui, |ui| {
                    for (i, val) in values_clone.iter().enumerate() {
                        let display_name = if val.name.is_empty() {
                            "(Default)".to_string()
                        } else {
                            val.name.clone()
                        };

                        let is_selected = self.selected_value == Some(i);
                        let name_resp = ui.selectable_label(is_selected, &display_name);

                        if name_resp.clicked() {
                            self.selected_value = Some(i);
                        }
                        if name_resp.double_clicked() {
                            edit_value = Some(i);
                        }

                        // Context menu on name
                        name_resp.context_menu(|ui| {
                            if ui.button("Edit Value...").clicked() {
                                edit_value = Some(i);
                                ui.close_menu();
                            }
                            if ui.button("Rename...").clicked() {
                                self.edit_dialog = EditDialog::RenameValue(
                                    val.name.clone(),
                                    val.name.clone(),
                                );
                                ui.close_menu();
                            }
                            if ui.button("Delete").clicked() {
                                delete_value = Some(i);
                                ui.close_menu();
                            }
                            ui.separator();
                            if ui.button("Copy Name").clicked() {
                                if let Ok(mut clipboard) = arboard::Clipboard::new() {
                                    clipboard.set_text(&display_name).ok();
                                }
                                ui.close_menu();
                            }
                            if ui.button("Copy Data").clicked() {
                                if let Ok(mut clipboard) = arboard::Clipboard::new() {
                                    clipboard.set_text(&val.data.display_data()).ok();
                                }
                                ui.close_menu();
                            }
                        });

                        ui.label(val.data.type_name());

                        let data_display = val.data.display_data();
                        let truncated = if data_display.len() > 200 {
                            format!("{}...", &data_display[..200])
                        } else {
                            data_display.clone()
                        };
                        ui.label(&truncated).on_hover_text(&data_display);

                        ui.end_row();
                    }
                });

            // Handle edit
            if let Some(i) = edit_value {
                if i < self.values.len() {
                    let val = &self.values[i];
                    let name = if val.name.is_empty() {
                        "(Default)".to_string()
                    } else {
                        val.name.clone()
                    };
                    self.edit_dialog = match &val.data {
                        RegValue::String(s) => {
                            EditDialog::EditStringValue(name, s.clone(), false)
                        }
                        RegValue::ExpandString(s) => {
                            EditDialog::EditStringValue(name, s.clone(), true)
                        }
                        RegValue::Dword(d) => {
                            EditDialog::EditDwordValue(name, format!("{}", d))
                        }
                        RegValue::Qword(q) => {
                            EditDialog::EditQwordValue(name, format!("{}", q))
                        }
                        RegValue::Binary(b) => EditDialog::EditBinaryValue(
                            name,
                            b.iter()
                                .map(|byte| format!("{:02x}", byte))
                                .collect::<Vec<_>>()
                                .join(" "),
                        ),
                        RegValue::MultiString(v) => {
                            EditDialog::EditMultiStringValue(name, v.join("\n"))
                        }
                        _ => EditDialog::None,
                    };
                }
            }

            // Handle delete
            if let Some(i) = delete_value {
                if i < self.values.len() {
                    let name = if self.values[i].name.is_empty() {
                        "(Default)".to_string()
                    } else {
                        self.values[i].name.clone()
                    };
                    self.edit_dialog = EditDialog::ConfirmDelete(name, false);
                }
            }
        });
    }

    fn show_dialogs(&mut self, ctx: &egui::Context) {
        let mut close_dialog = false;

        match self.edit_dialog.clone() {
            EditDialog::None => {}

            EditDialog::NewKey(ref _name) => {
                let mut name = if let EditDialog::NewKey(ref n) = self.edit_dialog {
                    n.clone()
                } else {
                    String::new()
                };

                egui::Window::new("New Key")
                    .collapsible(false)
                    .resizable(false)
                    .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                    .show(ctx, |ui| {
                        ui.horizontal(|ui| {
                            ui.label("Name:");
                            ui.text_edit_singleline(&mut name);
                        });
                        ui.horizontal(|ui| {
                            if ui.button("OK").clicked() {
                                if let Some(ref root) = self.selected_root.clone() {
                                    match self.store.create_key(root, &self.selected_path, &name) {
                                        Ok(()) => {
                                            self.status_message =
                                                format!("Created key: {} (pending sync)", name);
                                            self.maybe_auto_sync();
                                        }
                                        Err(e) => {
                                            self.error_message = Some(e);
                                        }
                                    }
                                }
                                close_dialog = true;
                            }
                            if ui.button("Cancel").clicked() {
                                close_dialog = true;
                            }
                        });
                    });
                if !close_dialog {
                    self.edit_dialog = EditDialog::NewKey(name);
                }
            }

            EditDialog::RenameValue(ref old, ref _new) => {
                let old = old.clone();
                let mut new_name = if let EditDialog::RenameValue(_, ref n) = self.edit_dialog {
                    n.clone()
                } else {
                    String::new()
                };

                egui::Window::new("Rename Value")
                    .collapsible(false)
                    .resizable(false)
                    .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                    .show(ctx, |ui| {
                        ui.horizontal(|ui| {
                            ui.label("New name:");
                            ui.text_edit_singleline(&mut new_name);
                        });
                        ui.horizontal(|ui| {
                            if ui.button("OK").clicked() {
                                if let Some(ref root) = self.selected_root.clone() {
                                    match self.store.rename_value(
                                        root,
                                        &self.selected_path,
                                        &old,
                                        &new_name,
                                    ) {
                                        Ok(()) => {
                                            self.refresh_values();
                                            self.status_message =
                                                format!("Renamed: {} -> {} (pending sync)", old, new_name);
                                            self.maybe_auto_sync();
                                            close_dialog = true;
                                        }
                                        Err(e) => {
                                            self.error_message = Some(e);
                                            close_dialog = true;
                                        }
                                    }
                                } else {
                                    close_dialog = true;
                                }
                            }
                            if ui.button("Cancel").clicked() {
                                close_dialog = true;
                            }
                        });
                    });
                if !close_dialog {
                    self.edit_dialog = EditDialog::RenameValue(old, new_name);
                }
            }

            EditDialog::EditStringValue(ref name, ref _data, is_expand) => {
                let name = name.clone();
                let mut data = if let EditDialog::EditStringValue(_, ref d, _) = self.edit_dialog {
                    d.clone()
                } else {
                    String::new()
                };

                let title = if is_expand {
                    "Edit Expandable String"
                } else {
                    "Edit String Value"
                };

                egui::Window::new(title)
                    .collapsible(false)
                    .resizable(true)
                    .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                    .show(ctx, |ui| {
                        ui.label(format!("Name: {}", name));
                        ui.label("Data:");
                        ui.text_edit_multiline(&mut data);
                        ui.horizontal(|ui| {
                            if ui.button("OK").clicked() {
                                if let Some(ref root) = self.selected_root.clone() {
                                    let val_name =
                                        if name == "(Default)" { "" } else { &name };
                                    let value = if is_expand {
                                        RegValue::ExpandString(data.clone())
                                    } else {
                                        RegValue::String(data.clone())
                                    };
                                    match self.store.set_value(
                                        root,
                                        &self.selected_path,
                                        val_name,
                                        &value,
                                    ) {
                                        Ok(()) => {
                                            self.refresh_values();
                                            self.status_message =
                                                format!("Updated: {} (pending sync)", name);
                                            self.maybe_auto_sync();
                                            close_dialog = true;
                                        }
                                        Err(e) => {
                                            self.error_message = Some(e);
                                            close_dialog = true;
                                        }
                                    }
                                } else {
                                    close_dialog = true;
                                }
                            }
                            if ui.button("Cancel").clicked() {
                                close_dialog = true;
                            }
                        });
                    });
                if !close_dialog {
                    self.edit_dialog = EditDialog::EditStringValue(name, data, is_expand);
                }
            }

            EditDialog::EditDwordValue(ref name, ref _data) => {
                let name = name.clone();
                let mut data = if let EditDialog::EditDwordValue(_, ref d) = self.edit_dialog {
                    d.clone()
                } else {
                    String::new()
                };

                egui::Window::new("Edit DWORD Value")
                    .collapsible(false)
                    .resizable(false)
                    .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                    .show(ctx, |ui| {
                        ui.label(format!("Name: {}", name));
                        ui.horizontal(|ui| {
                            ui.label("Value (decimal):");
                            ui.text_edit_singleline(&mut data);
                        });
                        ui.label(
                            egui::RichText::new("Enter decimal or prefix with 0x for hex")
                                .small()
                                .color(egui::Color32::GRAY),
                        );
                        ui.horizontal(|ui| {
                            if ui.button("OK").clicked() {
                                let parsed = if data.starts_with("0x") || data.starts_with("0X") {
                                    u32::from_str_radix(&data[2..], 16)
                                } else {
                                    data.parse::<u32>()
                                };
                                match parsed {
                                    Ok(d) => {
                                        if let Some(ref root) = self.selected_root.clone() {
                                            let val_name =
                                                if name == "(Default)" { "" } else { &name };
                                            match self.store.set_value(
                                                root,
                                                &self.selected_path,
                                                val_name,
                                                &RegValue::Dword(d),
                                            ) {
                                                Ok(()) => {
                                                    self.refresh_values();
                                                    self.status_message =
                                                        format!("Updated: {} (pending sync)", name);
                                                    self.maybe_auto_sync();
                                                }
                                                Err(e) => self.error_message = Some(e),
                                            }
                                        }
                                        close_dialog = true;
                                    }
                                    Err(_) => {
                                        self.error_message =
                                            Some("Invalid number format".to_string());
                                    }
                                }
                            }
                            if ui.button("Cancel").clicked() {
                                close_dialog = true;
                            }
                        });
                    });
                if !close_dialog {
                    self.edit_dialog = EditDialog::EditDwordValue(name, data);
                }
            }

            EditDialog::EditQwordValue(ref name, ref _data) => {
                let name = name.clone();
                let mut data = if let EditDialog::EditQwordValue(_, ref d) = self.edit_dialog {
                    d.clone()
                } else {
                    String::new()
                };

                egui::Window::new("Edit QWORD Value")
                    .collapsible(false)
                    .resizable(false)
                    .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                    .show(ctx, |ui| {
                        ui.label(format!("Name: {}", name));
                        ui.horizontal(|ui| {
                            ui.label("Value (decimal):");
                            ui.text_edit_singleline(&mut data);
                        });
                        ui.label(
                            egui::RichText::new("Enter decimal or prefix with 0x for hex")
                                .small()
                                .color(egui::Color32::GRAY),
                        );
                        ui.horizontal(|ui| {
                            if ui.button("OK").clicked() {
                                let parsed = if data.starts_with("0x") || data.starts_with("0X") {
                                    u64::from_str_radix(&data[2..], 16)
                                } else {
                                    data.parse::<u64>()
                                };
                                match parsed {
                                    Ok(q) => {
                                        if let Some(ref root) = self.selected_root.clone() {
                                            let val_name =
                                                if name == "(Default)" { "" } else { &name };
                                            match self.store.set_value(
                                                root,
                                                &self.selected_path,
                                                val_name,
                                                &RegValue::Qword(q),
                                            ) {
                                                Ok(()) => {
                                                    self.refresh_values();
                                                    self.status_message =
                                                        format!("Updated: {} (pending sync)", name);
                                                    self.maybe_auto_sync();
                                                }
                                                Err(e) => self.error_message = Some(e),
                                            }
                                        }
                                        close_dialog = true;
                                    }
                                    Err(_) => {
                                        self.error_message =
                                            Some("Invalid number format".to_string());
                                    }
                                }
                            }
                            if ui.button("Cancel").clicked() {
                                close_dialog = true;
                            }
                        });
                    });
                if !close_dialog {
                    self.edit_dialog = EditDialog::EditQwordValue(name, data);
                }
            }

            EditDialog::EditBinaryValue(ref name, ref _data) => {
                let name = name.clone();
                let mut data = if let EditDialog::EditBinaryValue(_, ref d) = self.edit_dialog {
                    d.clone()
                } else {
                    String::new()
                };

                egui::Window::new("Edit Binary Value")
                    .collapsible(false)
                    .resizable(true)
                    .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                    .show(ctx, |ui| {
                        ui.label(format!("Name: {}", name));
                        ui.label("Data (hex bytes, space-separated):");
                        ui.text_edit_multiline(&mut data);
                        ui.horizontal(|ui| {
                            if ui.button("OK").clicked() {
                                let bytes: Result<Vec<u8>, _> = data
                                    .split_whitespace()
                                    .map(|s| u8::from_str_radix(s, 16))
                                    .collect();
                                match bytes {
                                    Ok(b) => {
                                        if let Some(ref root) = self.selected_root.clone() {
                                            let val_name =
                                                if name == "(Default)" { "" } else { &name };
                                            match self.store.set_value(
                                                root,
                                                &self.selected_path,
                                                val_name,
                                                &RegValue::Binary(b),
                                            ) {
                                                Ok(()) => {
                                                    self.refresh_values();
                                                    self.status_message =
                                                        format!("Updated: {} (pending sync)", name);
                                                    self.maybe_auto_sync();
                                                }
                                                Err(e) => self.error_message = Some(e),
                                            }
                                        }
                                        close_dialog = true;
                                    }
                                    Err(_) => {
                                        self.error_message =
                                            Some("Invalid hex format".to_string());
                                    }
                                }
                            }
                            if ui.button("Cancel").clicked() {
                                close_dialog = true;
                            }
                        });
                    });
                if !close_dialog {
                    self.edit_dialog = EditDialog::EditBinaryValue(name, data);
                }
            }

            EditDialog::EditMultiStringValue(ref name, ref _data) => {
                let name = name.clone();
                let mut data =
                    if let EditDialog::EditMultiStringValue(_, ref d) = self.edit_dialog {
                        d.clone()
                    } else {
                        String::new()
                    };

                egui::Window::new("Edit Multi-String Value")
                    .collapsible(false)
                    .resizable(true)
                    .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                    .show(ctx, |ui| {
                        ui.label(format!("Name: {}", name));
                        ui.label("Data (one string per line):");
                        ui.text_edit_multiline(&mut data);
                        ui.horizontal(|ui| {
                            if ui.button("OK").clicked() {
                                let strings: Vec<String> = data
                                    .lines()
                                    .map(|l| l.to_string())
                                    .filter(|s| !s.is_empty())
                                    .collect();
                                if let Some(ref root) = self.selected_root.clone() {
                                    let val_name =
                                        if name == "(Default)" { "" } else { &name };
                                    match self.store.set_value(
                                        root,
                                        &self.selected_path,
                                        val_name,
                                        &RegValue::MultiString(strings),
                                    ) {
                                        Ok(()) => {
                                            self.refresh_values();
                                            self.status_message = format!("Updated: {} (pending sync)", name);
                                            self.maybe_auto_sync();
                                        }
                                        Err(e) => self.error_message = Some(e),
                                    }
                                }
                                close_dialog = true;
                            }
                            if ui.button("Cancel").clicked() {
                                close_dialog = true;
                            }
                        });
                    });
                if !close_dialog {
                    self.edit_dialog = EditDialog::EditMultiStringValue(name, data);
                }
            }

            EditDialog::NewValue(ref _name, type_idx) => {
                let mut name = if let EditDialog::NewValue(ref n, _) = self.edit_dialog {
                    n.clone()
                } else {
                    String::new()
                };
                let mut type_idx = type_idx;

                let types = [
                    "REG_SZ",
                    "REG_EXPAND_SZ",
                    "REG_DWORD",
                    "REG_QWORD",
                    "REG_BINARY",
                    "REG_MULTI_SZ",
                ];

                egui::Window::new("New Value")
                    .collapsible(false)
                    .resizable(false)
                    .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                    .show(ctx, |ui| {
                        ui.horizontal(|ui| {
                            ui.label("Name:");
                            ui.text_edit_singleline(&mut name);
                        });
                        ui.horizontal(|ui| {
                            ui.label("Type:");
                            egui::ComboBox::from_id_salt("new_value_type")
                                .selected_text(types[type_idx])
                                .show_ui(ui, |ui| {
                                    for (i, ty) in types.iter().enumerate() {
                                        if ui.selectable_label(type_idx == i, *ty).clicked() {
                                            type_idx = i;
                                        }
                                    }
                                });
                        });
                        ui.horizontal(|ui| {
                            if ui.button("OK").clicked() && !name.is_empty() {
                                if let Some(ref root) = self.selected_root.clone() {
                                    let default_value = match type_idx {
                                        0 => RegValue::String(String::new()),
                                        1 => RegValue::ExpandString(String::new()),
                                        2 => RegValue::Dword(0),
                                        3 => RegValue::Qword(0),
                                        4 => RegValue::Binary(Vec::new()),
                                        5 => RegValue::MultiString(Vec::new()),
                                        _ => RegValue::String(String::new()),
                                    };
                                    match self.store.set_value(
                                        root,
                                        &self.selected_path,
                                        &name,
                                        &default_value,
                                    ) {
                                        Ok(()) => {
                                            self.refresh_values();
                                            self.status_message =
                                                format!("Created value: {} (pending sync)", name);
                                            self.maybe_auto_sync();
                                        }
                                        Err(e) => self.error_message = Some(e),
                                    }
                                }
                                close_dialog = true;
                            }
                            if ui.button("Cancel").clicked() {
                                close_dialog = true;
                            }
                        });
                    });
                if !close_dialog {
                    self.edit_dialog = EditDialog::NewValue(name, type_idx);
                }
            }

            EditDialog::AddBookmark(ref _name, ref _notes, color_idx) => {
                let mut name = if let EditDialog::AddBookmark(ref n, _, _) = self.edit_dialog {
                    n.clone()
                } else {
                    String::new()
                };
                let mut notes =
                    if let EditDialog::AddBookmark(_, ref n, _) = self.edit_dialog {
                        n.clone()
                    } else {
                        String::new()
                    };
                let mut color_idx = color_idx;

                egui::Window::new("Add Bookmark")
                    .collapsible(false)
                    .resizable(false)
                    .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                    .show(ctx, |ui| {
                        ui.horizontal(|ui| {
                            ui.label("Name:");
                            ui.text_edit_singleline(&mut name);
                        });
                        ui.horizontal(|ui| {
                            ui.label("Notes:");
                            ui.text_edit_singleline(&mut notes);
                        });
                        ui.horizontal(|ui| {
                            ui.label("Color:");
                            let color_names: Vec<&str> = std::iter::once("None")
                                .chain(BookmarkColor::all().iter().map(|c| c.name()))
                                .collect();
                            egui::ComboBox::from_id_salt("bm_color")
                                .selected_text(color_names[color_idx])
                                .show_ui(ui, |ui| {
                                    for (i, cn) in color_names.iter().enumerate() {
                                        if ui.selectable_label(color_idx == i, *cn).clicked() {
                                            color_idx = i;
                                        }
                                    }
                                });
                        });
                        ui.horizontal(|ui| {
                            if ui.button("OK").clicked() {
                                let color = if color_idx == 0 {
                                    None
                                } else {
                                    Some(BookmarkColor::all()[color_idx - 1].clone())
                                };
                                self.store.add_bookmark(&Bookmark {
                                    name: name.clone(),
                                    path: self.full_path(),
                                    notes: notes.clone(),
                                    color,
                                });
                                self.status_message = format!("Bookmarked: {}", name);
                                close_dialog = true;
                            }
                            if ui.button("Cancel").clicked() {
                                close_dialog = true;
                            }
                        });
                    });
                if !close_dialog {
                    self.edit_dialog = EditDialog::AddBookmark(name, notes, color_idx);
                }
            }

            EditDialog::EditBookmark(idx, ref _name, ref _notes, color_idx) => {
                let mut name =
                    if let EditDialog::EditBookmark(_, ref n, _, _) = self.edit_dialog {
                        n.clone()
                    } else {
                        String::new()
                    };
                let mut notes =
                    if let EditDialog::EditBookmark(_, _, ref n, _) = self.edit_dialog {
                        n.clone()
                    } else {
                        String::new()
                    };
                let mut color_idx = color_idx;

                egui::Window::new("Edit Bookmark")
                    .collapsible(false)
                    .resizable(false)
                    .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                    .show(ctx, |ui| {
                        ui.horizontal(|ui| {
                            ui.label("Name:");
                            ui.text_edit_singleline(&mut name);
                        });
                        ui.horizontal(|ui| {
                            ui.label("Notes:");
                            ui.text_edit_singleline(&mut notes);
                        });
                        ui.horizontal(|ui| {
                            ui.label("Color:");
                            let color_names: Vec<&str> = std::iter::once("None")
                                .chain(BookmarkColor::all().iter().map(|c| c.name()))
                                .collect();
                            egui::ComboBox::from_id_salt("bm_color_edit")
                                .selected_text(color_names[color_idx])
                                .show_ui(ui, |ui| {
                                    for (i, cn) in color_names.iter().enumerate() {
                                        if ui.selectable_label(color_idx == i, *cn).clicked() {
                                            color_idx = i;
                                        }
                                    }
                                });
                        });
                        ui.horizontal(|ui| {
                            if ui.button("OK").clicked() {
                                let color = if color_idx == 0 {
                                    None
                                } else {
                                    Some(BookmarkColor::all()[color_idx - 1].clone())
                                };
                                let bookmarks = self.store.get_bookmarks();
                                if idx < bookmarks.len() {
                                    let path = bookmarks[idx].path.clone();
                                    self.store.update_bookmark(
                                        &path,
                                        &Bookmark {
                                            name: name.clone(),
                                            path: path.clone(),
                                            notes: notes.clone(),
                                            color,
                                        },
                                    );
                                }
                                close_dialog = true;
                            }
                            if ui.button("Cancel").clicked() {
                                close_dialog = true;
                            }
                        });
                    });
                if !close_dialog {
                    self.edit_dialog = EditDialog::EditBookmark(idx, name, notes, color_idx);
                }
            }

            EditDialog::ConfirmDelete(ref name, is_key) => {
                let name = name.clone();
                egui::Window::new("Confirm Delete")
                    .collapsible(false)
                    .resizable(false)
                    .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                    .show(ctx, |ui| {
                        if is_key {
                            ui.label(format!(
                                "Delete key '{}' and all its subkeys?",
                                name
                            ));
                        } else {
                            ui.label(format!("Delete value '{}'?", name));
                        }
                        ui.horizontal(|ui| {
                            if ui.button("Delete").clicked() {
                                if let Some(ref root) = self.selected_root.clone() {
                                    if is_key {
                                        // Delete the current key
                                        if let Some(pos) = self.selected_path.rfind('\\') {
                                            let parent = self.selected_path[..pos].to_string();
                                            match self.store.delete_key(root, &parent, &name) {
                                                Ok(()) => {
                                                    self.selected_path = parent.clone();
                                                    self.path_bar = format!(
                                                        "{}\\{}",
                                                        root, parent
                                                    );
                                                    self.refresh_values();
                                                    self.status_message =
                                                        format!("Deleted key: {} (pending sync)", name);
                                                    self.maybe_auto_sync();
                                                }
                                                Err(e) => self.error_message = Some(e),
                                            }
                                        } else {
                                            // Top-level key under root
                                            match self.store.delete_key(root, "", &name) {
                                                Ok(()) => {
                                                    self.selected_path = String::new();
                                                    self.path_bar = root.to_string();
                                                    self.refresh_values();
                                                    self.status_message =
                                                        format!("Deleted key: {} (pending sync)", name);
                                                    self.maybe_auto_sync();
                                                }
                                                Err(e) => self.error_message = Some(e),
                                            }
                                        }
                                    } else {
                                        let val_name =
                                            if name == "(Default)" { "" } else { &name as &str };
                                        match self.store.delete_value(
                                            root,
                                            &self.selected_path,
                                            val_name,
                                        ) {
                                            Ok(()) => {
                                                self.refresh_values();
                                                self.status_message =
                                                    format!("Deleted value: {} (pending sync)", name);
                                                self.maybe_auto_sync();
                                            }
                                            Err(e) => self.error_message = Some(e),
                                        }
                                    }
                                }
                                close_dialog = true;
                            }
                            if ui.button("Cancel").clicked() {
                                close_dialog = true;
                            }
                        });
                    });
            }

            EditDialog::SyncConflicts(ref conflicts) => {
                let conflicts = conflicts.clone();
                egui::Window::new("Sync Conflicts")
                    .collapsible(false)
                    .resizable(true)
                    .min_width(400.0)
                    .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                    .show(ctx, |ui| {
                        ui.label("The following changes could not be applied due to conflicts:");
                        ui.separator();
                        
                        egui::ScrollArea::vertical().max_height(300.0).show(ui, |ui| {
                            for conflict in &conflicts {
                                ui.horizontal(|ui| {
                                    ui.label("⚠");
                                    ui.label(conflict.change.description());
                                });
                                ui.label(
                                    egui::RichText::new(format!("  {:?}", conflict.conflict_type))
                                        .small()
                                        .color(egui::Color32::GRAY),
                                );
                                ui.separator();
                            }
                        });
                        
                        ui.horizontal(|ui| {
                            if ui.button("Force All").clicked() {
                                for conflict in &conflicts {
                                    self.store.force_push_change(&conflict.change).ok();
                                }
                                self.status_message = "Force-applied all conflicting changes".to_string();
                                self.refresh_values();
                                close_dialog = true;
                            }
                            if ui.button("Discard All").clicked() {
                                // Remove these changes from pending
                                for conflict in &conflicts {
                                    // Find and discard each
                                    for (id, c) in self.store.get_pending_changes() {
                                        if c == conflict.change {
                                            self.store.discard_pending_change(id);
                                            break;
                                        }
                                    }
                                }
                                self.status_message = "Discarded conflicting changes".to_string();
                                close_dialog = true;
                            }
                            if ui.button("Close").clicked() {
                                close_dialog = true;
                            }
                        });
                    });
            }

            EditDialog::ConfirmDiscardChanges => {
                egui::Window::new("Discard Changes")
                    .collapsible(false)
                    .resizable(false)
                    .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                    .show(ctx, |ui| {
                        let pending = self.store.pending_change_count();
                        ui.label(format!(
                            "Are you sure you want to discard {} pending changes?\n\nThis will reload the registry state.",
                            pending
                        ));
                        ui.horizontal(|ui| {
                            if ui.button("Discard All").clicked() {
                                self.store.discard_all_pending_changes();
                                self.store.pull_from_registry_async();
                                self.status_message = "Discarding and reloading from registry...".to_string();
                                close_dialog = true;
                            }
                            if ui.button("Cancel").clicked() {
                                close_dialog = true;
                            }
                        });
                    });
            }
        }

        if close_dialog {
            self.edit_dialog = EditDialog::None;
        }

        // Error dialog
        if let Some(ref err) = self.error_message.clone() {
            egui::Window::new("Error")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.colored_label(egui::Color32::from_rgb(255, 100, 100), err);
                    if ui.button("OK").clicked() {
                        self.error_message = None;
                    }
                });
        }
    }

    fn export_reg_file(&self) {
        if self.selected_root.is_none() {
            return;
        }
        if let Some(path) = rfd::FileDialog::new()
            .set_title("Export .reg file")
            .add_filter("Registry Files", &["reg"])
            .add_filter("All Files", &["*"])
            .save_file()
        {
            let mut content = String::from("Windows Registry Editor Version 5.00\r\n\r\n");
            let full_path = self.full_path();
            content.push_str(&format!("[{}]\r\n", full_path));

            for val in &self.values {
                let name_str = if val.name.is_empty() {
                    "@".to_string()
                } else {
                    format!("\"{}\"", val.name.replace('\\', "\\\\").replace('"', "\\\""))
                };

                let data_str = match &val.data {
                    RegValue::String(s) => {
                        format!(
                            "\"{}\"",
                            s.replace('\\', "\\\\").replace('"', "\\\"")
                        )
                    }
                    RegValue::ExpandString(s) => {
                        let hex: String = s
                            .encode_utf16()
                            .chain(std::iter::once(0))
                            .flat_map(|c| {
                                let bytes = c.to_le_bytes();
                                vec![
                                    format!("{:02x}", bytes[0]),
                                    format!("{:02x}", bytes[1]),
                                ]
                            })
                            .collect::<Vec<_>>()
                            .join(",");
                        format!("hex(2):{}", hex)
                    }
                    RegValue::Dword(d) => format!("dword:{:08x}", d),
                    RegValue::Qword(q) => {
                        let bytes = q.to_le_bytes();
                        let hex = bytes
                            .iter()
                            .map(|b| format!("{:02x}", b))
                            .collect::<Vec<_>>()
                            .join(",");
                        format!("hex(b):{}", hex)
                    }
                    RegValue::Binary(b) => {
                        let hex = b
                            .iter()
                            .map(|byte| format!("{:02x}", byte))
                            .collect::<Vec<_>>()
                            .join(",");
                        format!("hex:{}", hex)
                    }
                    RegValue::MultiString(strings) => {
                        let hex: String = strings
                            .iter()
                            .flat_map(|s| s.encode_utf16().chain(std::iter::once(0)))
                            .chain(std::iter::once(0))
                            .flat_map(|c| {
                                let bytes = c.to_le_bytes();
                                vec![
                                    format!("{:02x}", bytes[0]),
                                    format!("{:02x}", bytes[1]),
                                ]
                            })
                            .collect::<Vec<_>>()
                            .join(",");
                        format!("hex(7):{}", hex)
                    }
                    _ => continue,
                };

                content.push_str(&format!("{}={}\r\n", name_str, data_str));
            }

            std::fs::write(path, content).ok();
        }
    }

    fn import_reg_file(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .set_title("Import .reg file")
            .add_filter("Registry Files", &["reg"])
            .add_filter("All Files", &["*"])
            .pick_file()
        {
            match std::fs::read_to_string(&path) {
                Ok(content) => {
                    // Basic .reg file parser
                    let mut current_key: Option<(RootKey, String)> = None;
                    let mut imported = 0;
                    let mut errors = 0;

                    for line in content.lines() {
                        let line = line.trim();
                        if line.is_empty()
                            || line.starts_with("Windows Registry Editor")
                            || line.starts_with(';')
                        {
                            continue;
                        }

                        if line.starts_with('[') && line.ends_with(']') {
                            let key_path = &line[1..line.len() - 1];
                            let parts: Vec<&str> = key_path.splitn(2, '\\').collect();
                            if let Some(root) = RootKey::from_name(parts[0]) {
                                let sub = if parts.len() > 1 { parts[1] } else { "" };
                                // Ensure key exists
                                if !sub.is_empty() {
                                    registry::create_key(
                                        &root,
                                        "",
                                        sub,
                                    )
                                    .ok();
                                }
                                current_key = Some((root, sub.to_string()));
                            }
                            continue;
                        }

                        if let Some((ref root, ref path)) = current_key {
                            // Parse value line: "name"=value or @=value
                            if let Some(eq_pos) = line.find('=') {
                                let name_part = &line[..eq_pos];
                                let data_part = &line[eq_pos + 1..];

                                let name = if name_part == "@" {
                                    String::new()
                                } else if name_part.starts_with('"') && name_part.ends_with('"')
                                {
                                    name_part[1..name_part.len() - 1]
                                        .replace("\\\\", "\\")
                                        .replace("\\\"", "\"")
                                } else {
                                    continue;
                                };

                                let value = if data_part.starts_with('"') {
                                    // String value
                                    let s = &data_part[1..data_part.len().saturating_sub(1)];
                                    Some(RegValue::String(
                                        s.replace("\\\\", "\\").replace("\\\"", "\""),
                                    ))
                                } else if data_part.starts_with("dword:") {
                                    u32::from_str_radix(&data_part[6..], 16)
                                        .ok()
                                        .map(RegValue::Dword)
                                } else {
                                    // Skip complex hex values for now
                                    None
                                };

                                if let Some(val) = value {
                                    match registry::set_value(root, path, &name, &val) {
                                        Ok(()) => imported += 1,
                                        Err(_) => errors += 1,
                                    }
                                }
                            }
                        }
                    }

                    self.status_message = format!(
                        "Import complete: {} values imported, {} errors. Refreshing...",
                        imported, errors
                    );
                    self.store.pull_from_registry_async();
                }
                Err(e) => {
                    self.error_message = Some(format!("Failed to read file: {}", e));
                }
            }
        }
    }
}

/// Actions for bookmark management in the bookmarks panel.
enum BookmarkAction {
    Remove(usize),
    MoveUp(usize),
    MoveDown(usize),
    Edit(usize, String, String, usize),
}

impl eframe::App for RegistryEditorApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Handle keyboard shortcuts
        if ctx.input(|i| i.key_pressed(egui::Key::F5)) {
            self.store.pull_from_registry_async();
            self.status_message = "Refreshing from registry...".to_string();
        }
        if ctx.input(|i| i.modifiers.ctrl && i.key_pressed(egui::Key::F)) {
            self.active_panel = Panel::Search;
        }
        if ctx.input(|i| i.modifiers.ctrl && i.key_pressed(egui::Key::B)) {
            self.active_panel = Panel::Bookmarks;
        }
        if ctx.input(|i| i.modifiers.ctrl && i.key_pressed(egui::Key::D)) {
            let path = self.full_path();
            if !path.is_empty() {
                if self.store.is_bookmarked(&path) {
                    self.store.remove_bookmark(&path);
                    self.status_message = "Bookmark removed".to_string();
                } else {
                    let name = path.rsplit('\\').next().unwrap_or(&path).to_string();
                    self.store.add_bookmark(&Bookmark {
                        name,
                        path: path.clone(),
                        notes: String::new(),
                        color: None,
                    });
                    self.status_message = "Bookmark added".to_string();
                }
            }
        }

        // Top panel - menu & path bar
        egui::TopBottomPanel::top("top_panel").show(ctx, |ui| {
            self.show_menu_bar(ui);
            ui.separator();
            self.show_path_bar(ui);
        });

        // Bottom panel - status bar
        egui::TopBottomPanel::bottom("status_bar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                // Left side: status message
                ui.label(&self.status_message);
                
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // Keyboard shortcuts (far right)
                    ui.label(
                        egui::RichText::new("Ctrl+F: Search | Ctrl+B: Bookmarks | F5: Refresh")
                            .small()
                            .color(egui::Color32::GRAY),
                    );
                    
                    ui.separator();
                    
                    // Sync status indicator
                    let is_syncing = self.store.is_syncing.load(Ordering::Relaxed);
                    let pending = self.store.pending_change_count();
                    
                    if is_syncing {
                        let progress = self.store.sync_progress.load(Ordering::Relaxed);
                        let total = self.store.sync_total.load(Ordering::Relaxed);
                        ui.label(
                            egui::RichText::new(format!("⟳ Syncing... {}/{}", progress, total))
                                .color(egui::Color32::from_rgb(100, 180, 255)),
                        );
                    } else if pending > 0 {
                        ui.label(
                            egui::RichText::new(format!("● {} pending", pending))
                                .color(egui::Color32::from_rgb(255, 180, 100)),
                        );
                    } else {
                        ui.label(
                            egui::RichText::new("✓ Ready")
                                .color(egui::Color32::from_rgb(100, 200, 100)),
                        );
                    }
                });
            });
        });

        // Left panel - tree / search / bookmarks
        egui::SidePanel::left("left_panel")
            .default_width(350.0)
            .min_width(200.0)
            .resizable(true)
            .show(ctx, |ui| {
                self.show_left_panel(ui);
            });

        // Central panel - values
        egui::CentralPanel::default().show(ctx, |ui| {
            self.show_values_panel(ui);
        });

        // Dialogs
        self.show_dialogs(ctx);
    }
}

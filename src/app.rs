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
use rust_hive::sync::{DebugCategory, PendingChange, SyncConflict, SyncStore};

// External crate imports
use eframe::egui;
use std::sync::atomic::Ordering;
use std::time::Instant;

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
    
    /// Flag to reset values scroll area to top-left on next frame
    reset_values_scroll: bool,
    
    /// Flag to reset search results scroll area to top-left on next frame
    reset_search_scroll: bool,
    
    /// Flag to scroll the tree to show the selected key
    scroll_tree_to_selected: bool,
    
    /// Frames remaining to attempt tree scroll (timeout protection)
    scroll_tree_frames_remaining: u8,

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
    
    /// The last search query that was executed.
    /// Used to avoid re-running the same search on Enter.
    last_executed_query: String,
    
    /// Timestamp of the last keystroke in the search box (for debouncing).
    /// Search starts 500ms after the user stops typing.
    search_debounce_start: Option<Instant>,
    
    /// The query text when debounce timer started.
    /// If this differs from current query, timer resets.
    search_debounce_query: String,

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
    
    /// Track if we were syncing last frame (to detect completion)
    was_syncing: bool,

    /// Track if we had pending fetches last frame (to detect completion)
    had_pending_fetches: bool,

    /// Track if we were searching last frame (to detect completion)
    was_searching_last_frame: bool,

    /// Whether the debug overlay window is open
    show_profiler: bool,

    /// Whether the debug log window is open (non-modal)
    show_debug_log: bool,

    /// Filter for debug log categories (None = show all)
    debug_log_filter: Option<DebugCategory>,

    // ── Debug / profiling stats ───────────────────────────────────────────────
    /// Wall-clock time of the last frame start (for frame-time measurement)
    last_frame_start: Option<Instant>,
    /// How long the last full update() took
    last_frame_ms: f32,
    /// How long render_tree_nodes() took last frame
    last_tree_render_us: u64,
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
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        // Clear any stale egui widget state from previous sessions.
        // Without this, expanding keys with thousands of subkeys (e.g., HKCR) causes egui
        // to persist tens of thousands of CollapsingHeader states to app.ron (~600KB+),
        // which makes the app unresponsive or invisible on the next launch.
        // Tree expand/collapse state is managed by our own `expanded_keys` set, so we
        // don't need egui's persistence for it.
        cc.egui_ctx.memory_mut(|m| m.data.clear());

        // Create the store - this opens/creates the SQLite database
        // Settings are automatically loaded from the database
        let store = SyncStore::new();
        
        // Optimize database on startup - updates query planner statistics
        store.optimize();
        
        // Load sync_mode preference
        let sync_mode = match store.load_setting("sync_mode").as_deref() {
            Some("auto") => SyncMode::AutoSync,
            _ => SyncMode::Manual,
        };

        Self {
            // Navigation - start with nothing selected
            selected_root: None,
            selected_path: String::new(),
            expanded_keys: std::collections::HashSet::new(),
            
            // Values - empty until a key is selected
            values: Vec::new(),
            selected_value: None,
            reset_values_scroll: false,
            reset_search_scroll: false,
            scroll_tree_to_selected: false,
            scroll_tree_frames_remaining: 0,
            
            // Search - default options, new state
            search_options: SearchOptions::default(),
            search_state: SearchState::new(),
            search_results_snapshot: Vec::new(),
            last_executed_query: String::new(),
            search_debounce_start: None,
            search_debounce_query: String::new(),
            
            // Backend
            store,
            sync_mode,
            
            // UI state
            active_panel: Panel::Tree,
            path_bar: String::new(),
            status_message: String::new(),
            edit_dialog: EditDialog::None,
            error_message: None,
            show_search_options: false,
            was_syncing: false,
            had_pending_fetches: false,
            was_searching_last_frame: false,
            show_profiler: false,
            show_debug_log: false,
            debug_log_filter: None,
            last_frame_start: None,
            last_frame_ms: 0.0,
            last_tree_render_us: 0,
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
        self.navigate_to_path_internal(full_path, true);
    }

    /// Navigate to a path, optionally checking if it exists first.
    /// Use check_exists=false for search results (they came from SQLite, so they exist).
    fn navigate_to_path_internal(&mut self, full_path: &str, check_exists: bool) {
        // Trim trailing backslash if present (e.g., "HKEY_CURRENT_USER\Software\" -> "HKEY_CURRENT_USER\Software")
        let full_path = full_path.trim_end_matches('\\');
        
        // Parse "HKEY_XXX\path\to\key"
        let parts: Vec<&str> = full_path.splitn(2, '\\').collect();
        let root_name = parts[0];
        let sub_path = if parts.len() > 1 { parts[1] } else { "" };

        if let Some(root) = RootKey::from_name(root_name) {
            // Optionally verify the key exists in our SQLite store
            if check_exists && !sub_path.is_empty() {
                if !self.store.key_exists(&root, sub_path) {
                    // Key doesn't exist — treat input as a search
                    self.run_path_bar_search(full_path);
                    return;
                }
            }

            // Expand all parent keys and pre-fetch their subkeys
            let mut cumulative = root.to_string();
            self.expanded_keys.insert(cumulative.clone());
            // Pre-fetch root key's subkeys
            self.store.fetch_subkeys_async(&root, "");
            
            if !sub_path.is_empty() {
                let mut parent_path = String::new();
                for segment in sub_path.split('\\') {
                    // Pre-fetch this parent's subkeys so tree can render children immediately
                    if !parent_path.is_empty() {
                        self.store.fetch_subkeys_async(&root, &parent_path);
                    }
                    
                    cumulative = format!("{}\\{}", cumulative, segment);
                    self.expanded_keys.insert(cumulative.clone());
                    
                    // Build parent path for next iteration
                    if parent_path.is_empty() {
                        parent_path = segment.to_string();
                    } else {
                        parent_path = format!("{}\\{}", parent_path, segment);
                    }
                }
                // Also fetch the final selected key's subkeys (in case it has children)
                self.store.fetch_subkeys_async(&root, &parent_path);
            }

            self.selected_root = Some(root.clone());
            self.selected_path = sub_path.to_string();
            self.path_bar = full_path.to_string();
            self.refresh_values();
            self.reset_values_scroll = true;
            self.scroll_tree_to_selected = true;
            self.scroll_tree_frames_remaining = 30;  // Timeout after ~0.5s at 60fps
            
            // When navigating from path bar (check_exists=true), switch to Tree panel
            // so user can see the key's location in the hierarchy.
            // When navigating from search results (check_exists=false), stay on search
            // unless the key has no values.
            if check_exists {
                // From path bar - always switch to tree
                self.active_panel = Panel::Tree;
            } else {
                // From search - only switch if key has no values
                let has_cached = self.store.has_cached_values(&root, sub_path);
                if has_cached && self.values.is_empty() {
                    self.active_panel = Panel::Tree;
                }
            }
        } else {
            // Doesn't start with a valid root — treat as search
            self.run_path_bar_search(full_path);
        }
    }

    fn run_path_bar_search(&mut self, query: &str) {
        self.search_options.query = query.to_string();
        self.last_executed_query = query.to_string();
        self.search_results_snapshot.clear();
        self.active_panel = Panel::Search;
        // Use SQLite-based search from the sync store
        rust_hive::search::start_search_with_store(
            self.search_options.clone(),
            self.search_state.clone(),
            self.store.clone(),
            true, // fallback to live registry if SQLite finds nothing
        );
        self.status_message = format!("Searching for \"{}\"...", query);
    }

    fn refresh_values(&mut self) {
        if let Some(ref root) = self.selected_root {
            // Use non-blocking version - only returns cached data
            self.values = self.store.get_values_cached_only(root, &self.selected_path);
            
            // If no cached data, trigger async fetch (won't block UI)
            if self.values.is_empty() && !self.store.has_cached_values(root, &self.selected_path) {
                self.store.fetch_values_async(root, &self.selected_path);
            }
            self.selected_value = None;
        }
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
                if ui.button("Open .reg file...").clicked() {
                    self.import_reg_file();
                    ui.close_menu();
                }
                if ui.button("Save .reg file...").clicked() {
                    self.export_reg_file();
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
                ui.separator();
                if ui.checkbox(&mut self.show_profiler, "Debug Overlay").clicked() {
                    ui.close_menu();
                }
            });

            // Sync menu - just the sync actions
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
            });

            // Tools menu with Options
            ui.menu_button("Tools", |ui| {
                ui.menu_button("Options", |ui| {
                    ui.label(egui::RichText::new("Sync Mode").strong());
                    if ui.radio(self.sync_mode == SyncMode::Manual, "Manual - stage changes before pushing").clicked() {
                        self.sync_mode = SyncMode::Manual;
                        self.store.save_setting("sync_mode", "manual");
                    }
                    if ui.radio(self.sync_mode == SyncMode::AutoSync, "Auto-sync - push changes immediately").clicked() {
                        self.sync_mode = SyncMode::AutoSync;
                        self.store.save_setting("sync_mode", "auto");
                    }
                    
                    ui.separator();
                    ui.label(egui::RichText::new("Background Sync").strong());
                    let mut auto_pull = self.store.auto_pull_enabled.load(Ordering::Relaxed);
                    if ui.checkbox(&mut auto_pull, "Enable background sync from registry").changed() {
                        self.store.auto_pull_enabled.store(auto_pull, Ordering::SeqCst);
                        self.store.save_auto_pull_enabled();
                    }
                });
                
                ui.separator();
                
                // Debug mode toggle
                let debug_enabled = self.store.debug_enabled.load(Ordering::Relaxed);
                let mut debug_mode = debug_enabled;
                if ui.checkbox(&mut debug_mode, "Debug Mode").changed() {
                    self.store.debug_enabled.store(debug_mode, Ordering::SeqCst);
                    if debug_mode {
                        self.show_debug_log = true;
                    }
                }
                if debug_enabled {
                    if ui.button("Open Debug Log...").clicked() {
                        self.show_debug_log = true;
                        ui.close_menu();
                    }
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
            
            // Detect typing in path bar - switch to search mode
            let path_changed = self.path_bar != self.search_debounce_query;
            if path_changed && !self.path_bar.is_empty() {
                // Check if this looks like a registry path (contains backslash or starts with HK)
                let looks_like_path = self.path_bar.contains('\\') 
                    || self.path_bar.to_uppercase().starts_with("HK");
                
                if !looks_like_path {
                    // Treat as search query - switch to search panel and start debounce
                    self.active_panel = Panel::Search;
                    self.search_options.query = self.path_bar.clone();
                    self.search_debounce_query = self.path_bar.clone();
                    self.search_debounce_start = Some(Instant::now());
                    
                    // Cancel any running search
                    if self.search_state.is_searching.load(Ordering::Relaxed) {
                        self.search_state.cancel.store(true, Ordering::SeqCst);
                    }
                }
            }
            
            if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                let path = self.path_bar.clone();
                // Check if it looks like a path or a search
                let looks_like_path = path.contains('\\') 
                    || path.to_uppercase().starts_with("HK");
                
                if looks_like_path {
                    self.navigate_to_path(&path);
                } else if !path.is_empty() {
                    // Treat as immediate search
                    self.active_panel = Panel::Search;
                    self.search_options.query = path.clone();
                    self.search_debounce_query = path.clone();
                    self.search_debounce_start = None;
                    self.last_executed_query = path;
                    self.search_results_snapshot.clear();
                    rust_hive::search::start_search_with_store(
                        self.search_options.clone(),
                        self.search_state.clone(),
                        self.store.clone(),
                        true,
                    );
                }
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
        // Handle scroll timeout
        if self.scroll_tree_to_selected && self.scroll_tree_frames_remaining > 0 {
            self.scroll_tree_frames_remaining -= 1;
            if self.scroll_tree_frames_remaining == 0 {
                // Timeout - give up on scrolling
                self.scroll_tree_to_selected = false;
            }
        }
        
        egui::ScrollArea::both().show(ui, |ui| {
            let mut nodes_to_render: Vec<(RootKey, String)> = Vec::new();
            for root in RootKey::all() {
                nodes_to_render.push((root.clone(), String::new()));
            }
            let t = Instant::now();
            self.render_tree_nodes(ui, &nodes_to_render);
            self.last_tree_render_us = t.elapsed().as_micros() as u64;
            
            // Keep repainting while waiting to scroll to selected node
            // (node may not be rendered yet because parent subkeys aren't fetched)
            if self.scroll_tree_to_selected {
                ui.ctx().request_repaint();
            }
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

            // Single mutex acquire: returns (subkeys, is_fetched).
            // Root keys (path="") are NOT treated as pre-fetched — they must be fetched
            // the same way as any other key when first opened.
            let (subkeys, is_fetched) = self.store.get_subkeys_cached(root, path);
            
            // Trigger fetch immediately when rendering an unfetched non-root node.
            // This ensures we know the node's leaf status by the next frame.
            // The fetch is a no-op if already in-flight.
            if !is_fetched && !path.is_empty() {
                self.store.fetch_subkeys_async(root, path);
            }
            
            // Determine if this node should show an expand arrow:
            // - Root keys always have children (show arrow)
            // - Nodes with children in cache (show arrow)
            // - Unfetched nodes (show arrow - optimistic, might have children)
            // - Fetched nodes with no children: confirmed leaf (no arrow)
            let is_root_key = path.is_empty();
            let is_confirmed_leaf = is_fetched && subkeys.is_empty() && !is_root_key;

            if !is_confirmed_leaf {
                // Node with children (or might have children) - show expand arrow
                let should_be_open = self.expanded_keys.contains(&full_key);
                let id = ui.make_persistent_id(&full_key);
                
                // Style selected nodes with color and bold
                let label_text = if is_selected {
                    egui::RichText::new(format!("📁 {}", display_name))
                        .strong()
                        .color(egui::Color32::from_rgb(100, 200, 255))
                } else {
                    egui::RichText::new(&display_name)
                };
                
                let header = egui::CollapsingHeader::new(label_text)
                .id_salt(id)
                .default_open(should_be_open)
                .open(if should_be_open { Some(true) } else { None });
                
                let resp = header.show(ui, |ui| {
                    // Fetch when node is open (needed for root nodes)
                    if !is_fetched {
                        self.store.fetch_subkeys_async(root, path);
                    }
                    
                    // Cap visible children to avoid egui laying out thousands of widgets
                    const MAX_CHILDREN: usize = 500;
                    let total = subkeys.len();
                    let visible = subkeys.iter().take(MAX_CHILDREN);

                    let child_nodes: Vec<(RootKey, String)> = visible
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

                    if total > MAX_CHILDREN {
                        ui.label(
                            egui::RichText::new(format!(
                                "… {} more keys (use search to navigate)",
                                total - MAX_CHILDREN
                            ))
                            .small()
                            .color(egui::Color32::GRAY),
                        );
                    }
                });

                // Track expansion state based on fully open/closed, not animation
                if resp.fully_open() {
                    self.expanded_keys.insert(full_key.clone());
                } else if resp.fully_closed() {
                    self.expanded_keys.remove(&full_key);
                }

                // Handle click on header
                if resp.header_response.clicked() {
                    self.selected_root = Some(root.clone());
                    self.selected_path = path.to_string();
                    self.path_bar = full_key.clone();
                    self.refresh_values();
                }
                
                // Scroll to selected item if requested
                if is_selected && self.scroll_tree_to_selected {
                    resp.header_response.scroll_to_me(Some(egui::Align::Center));
                    self.scroll_tree_to_selected = false;
                }

                // Context menu
                self.tree_node_context_menu(&resp.header_response, root, path, &full_key);
            } else {
                // Confirmed leaf node - no expand arrow, clickable label
                // Add horizontal spacing to align with CollapsingHeader text (arrow width ~19px)
                let text = if is_selected {
                    egui::RichText::new(format!("    📄 {}", display_name))
                        .strong()
                        .color(egui::Color32::from_rgb(100, 200, 255))
                } else {
                    egui::RichText::new(format!("    {}", display_name))
                };
                let resp = ui.add(egui::Label::new(text).sense(egui::Sense::click()));
                
                // Select on click
                if resp.clicked() {
                    self.selected_root = Some(root.clone());
                    self.selected_path = path.to_string();
                    self.path_bar = full_key.clone();
                    self.refresh_values();
                }
                
                // Scroll to selected item if requested
                if is_selected && self.scroll_tree_to_selected {
                    resp.scroll_to_me(Some(egui::Align::Center));
                    self.scroll_tree_to_selected = false;
                }
                
                // If this node is selected but values haven't loaded yet, keep trying
                // (handles race condition where click happens during node type transition)
                if is_selected && self.values.is_empty() {
                    self.refresh_values();
                }
                
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
        let is_searching = self.search_state.is_searching.load(Ordering::Relaxed);
        
        // Show current search query and cancel button
        ui.horizontal(|ui| {
            if !self.search_options.query.is_empty() {
                ui.label(format!("Searching: \"{}\"", self.search_options.query));
            } else {
                ui.label("Type in the path bar to search");
            }
            
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if is_searching {
                    if ui.button("Cancel").clicked() {
                        self.search_state.cancel.store(true, Ordering::SeqCst);
                    }
                }
            });
        });
        
        // Allow canceling search with Escape key
        if is_searching && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.search_state.cancel.store(true, Ordering::SeqCst);
        }

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
                        self.store.save_auto_pull_enabled();
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
                                self.store.save_auto_pull_interval();
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
                            self.store.save_pull_max_depth();
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
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(format!(
                    "Searching... {} keys scanned, {} found",
                    scanned, count
                ));
            });
            ui.ctx().request_repaint();
            // Don't show results while searching - wait until complete
        } else if !self.last_executed_query.is_empty() {
            // Search completed - update snapshot with final results (only once when search finishes)
            if self.was_searching_last_frame {
                self.search_results_snapshot = self.search_state.results.lock().unwrap().clone();
                self.reset_search_scroll = true;  // Scroll to left when new search completes
            }
            
            if self.search_results_snapshot.is_empty() {
                ui.label(format!("No results for \"{}\"", self.last_executed_query));
            } else {
                ui.label(format!("{} results", self.search_results_snapshot.len()));
            }
        }

        // Results list - only show when not searching
        let mut navigate_to: Option<String> = None;
        let mut bookmark_path: Option<String> = None;
        
        if !is_searching {
            let scroll_to_left = self.reset_search_scroll;
            self.reset_search_scroll = false;
            
            let mut scroll_area = egui::ScrollArea::both()
                .id_salt("search_results_scroll");
            
            // Reset horizontal scroll only (not vertical) when clicking a result
            if scroll_to_left {
                scroll_area = scroll_area.horizontal_scroll_offset(0.0);
            }
            
            scroll_area.show(ui, |ui| {
                for (idx, result) in self.search_results_snapshot.iter().enumerate() {
                    let icon = match result.match_type {
                        MatchType::KeyName => "🔑",
                        MatchType::ValueName => "📝",
                        MatchType::ValueData => "📄",
                    };

                    // Alternating background for better visual separation
                    let bg_color = if idx % 2 == 0 {
                        egui::Color32::from_rgba_unmultiplied(60, 60, 80, 255)
                    } else {
                        egui::Color32::from_rgba_unmultiplied(40, 40, 55, 255)
                    };

                    let text_color = match result.match_type {
                        MatchType::KeyName => egui::Color32::from_rgb(100, 180, 255),
                        MatchType::ValueName => egui::Color32::from_rgb(100, 255, 100),
                        MatchType::ValueData => egui::Color32::from_rgb(255, 200, 100),
                    };

                    let full_path = result.full_path();
                    let is_bookmarked = self.store.is_bookmarked(&full_path);

                    egui::Frame::new()
                        .fill(bg_color)
                        .inner_margin(egui::Margin::symmetric(6, 4))
                        .outer_margin(egui::Margin::symmetric(0, 1))
                        .stroke(egui::Stroke::new(1.0, egui::Color32::from_gray(70)))
                        .corner_radius(3.0)
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(icon);
                                
                                let path_text = result.full_path();
                                let display_text = if let Some(ref vname) = result.value_name {
                                    format!(
                                        "{}\\{} = {}",
                                        path_text,
                                        vname,
                                        result.value_data.as_deref().unwrap_or("")
                                    )
                                } else {
                                    path_text.clone()
                                };

                                let resp = ui.add(
                                    egui::Label::new(
                                        egui::RichText::new(&display_text).small().color(text_color),
                                    )
                                    .sense(egui::Sense::click()),
                                );

                                if resp.clicked() {
                                    navigate_to = Some(result.full_path());
                                }

                                // Context menu on right-click
                                resp.context_menu(|ui| {
                                    if is_bookmarked {
                                        if ui.button("Remove Bookmark").clicked() {
                                            self.store.remove_bookmark(&full_path);
                                            self.status_message = "Bookmark removed".to_string();
                                            ui.close_menu();
                                        }
                                    } else {
                                        if ui.button("Add Bookmark").clicked() {
                                            bookmark_path = Some(full_path.clone());
                                            ui.close_menu();
                                        }
                                    }
                                    
                                    ui.separator();
                                    
                                    if ui.button("Copy Path").clicked() {
                                        if let Ok(mut clipboard) = arboard::Clipboard::new() {
                                            clipboard.set_text(&full_path).ok();
                                        }
                                        self.status_message = "Path copied to clipboard".to_string();
                                        ui.close_menu();
                                    }
                                });

                                resp.on_hover_text(format!(
                                    "Match: {}\nPath: {}",
                                    result.match_type,
                                    result.full_path()
                                ));
                            });
                        });
                }
            });
        }
        
        // Handle bookmark addition outside the closure
        if let Some(path) = bookmark_path {
            let name = path.rsplit('\\').next().unwrap_or(&path).to_string();
            self.store.add_bookmark(&Bookmark {
                name,
                path,
                notes: String::new(),
                color: None,
            });
            self.status_message = "Bookmark added".to_string();
        }
        
        // Handle navigation outside the scroll area closure
        if let Some(path) = navigate_to {
            // Ensure parent keys are cached so the tree can display them
            self.ensure_path_cached(&path);
            self.navigate_to_path_internal(&path, false);
            // Scroll search results to the left (key names can be very long)
            self.reset_search_scroll = true;
            // Stay on search panel - don't switch to tree
        }
    }
    
    /// Ensure all keys in a path are cached in SQLite so the tree can display them.
    fn ensure_path_cached(&self, full_path: &str) {
        let parts: Vec<&str> = full_path.splitn(2, '\\').collect();
        let root_name = parts[0];
        let sub_path = if parts.len() > 1 { parts[1] } else { "" };
        
        if let Some(root) = RootKey::from_name(root_name) {
            // Cache each segment of the path
            let mut cumulative = String::new();
            for segment in sub_path.split('\\') {
                if segment.is_empty() {
                    continue;
                }
                let parent = cumulative.clone();
                if cumulative.is_empty() {
                    cumulative = segment.to_string();
                } else {
                    cumulative = format!("{}\\{}", cumulative, segment);
                }
                // Trigger a fetch for this path segment (will cache if not already)
                self.store.fetch_subkeys_async(&root, &parent);
            }
        }
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

        // Values list - use id_salt so we can reset scroll position
        let scroll_to_top = self.reset_values_scroll;
        self.reset_values_scroll = false;
        
        egui::ScrollArea::both()
            .id_salt("values_scroll")
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible)
            .show(ui, |ui| {
                // Reset scroll to top-left if requested
                if scroll_to_top {
                    ui.scroll_to_cursor(Some(egui::Align::Min));
                }
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
        let root = match &self.selected_root {
            Some(r) => r.clone(),
            None => return,
        };
        if let Some(file_path) = rfd::FileDialog::new()
            .set_title("Save .reg file")
            .add_filter("Registry Files", &["reg"])
            .add_filter("All Files", &["*"])
            .save_file()
        {
            let mut content = String::from("Windows Registry Editor Version 5.00\r\n\r\n");
            
            // Recursively export the selected key and all children
            let mut keys_to_export: Vec<String> = vec![self.selected_path.clone()];
            let mut processed: std::collections::HashSet<String> = std::collections::HashSet::new();
            
            while let Some(current_path) = keys_to_export.pop() {
                if processed.contains(&current_path) {
                    continue;
                }
                processed.insert(current_path.clone());
                
                // Build full path for this key
                let full_path = if current_path.is_empty() {
                    root.to_string()
                } else {
                    format!("{}\\{}", root, current_path)
                };
                
                content.push_str(&format!("[{}]\r\n", full_path));
                
                // Get values for this key
                if let Ok(values) = self.store.get_values(&root, &current_path) {
                    for val in &values {
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
                }
                
                content.push_str("\r\n");
                
                // Get subkeys and add them to the list to process
                let subkeys = self.store.get_subkeys(&root, &current_path);
                for subkey in subkeys {
                    let child_path = if current_path.is_empty() {
                        subkey
                    } else {
                        format!("{}\\{}", current_path, subkey)
                    };
                    keys_to_export.push(child_path);
                }
            }

            std::fs::write(file_path, content).ok();
        }
    }

    fn import_reg_file(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .set_title("Open .reg file")
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

/// Format bytes as human-readable string (KB, MB, GB).
fn format_bytes(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{} B", bytes)
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else if bytes < 1024 * 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else {
        format!("{:.2} GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    }
}

/// Get the current process's working set (memory usage) in bytes.
#[cfg(windows)]
fn get_process_memory_bytes() -> u64 {
    use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
    use windows::Win32::System::Threading::GetCurrentProcess;
    
    unsafe {
        let handle = GetCurrentProcess();
        let mut pmc: PROCESS_MEMORY_COUNTERS = std::mem::zeroed();
        pmc.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
        
        if GetProcessMemoryInfo(handle, &mut pmc, pmc.cb).is_ok() {
            pmc.WorkingSetSize as u64
        } else {
            0
        }
    }
}

#[cfg(not(windows))]
fn get_process_memory_bytes() -> u64 {
    0 // Not implemented for non-Windows
}

/// Shows a floating debug overlay with live performance metrics.
impl RegistryEditorApp {
    fn show_debug_overlay(&self, ctx: &egui::Context) {
        let in_flight = self.store.pending_fetches.load(Ordering::Relaxed);
        let cached_keys = self.store.subkey_cache_len();
        let db_size_bytes = self.store.get_db_size_bytes();
        let memory_bytes = get_process_memory_bytes();

        egui::Window::new("Debug Overlay")
            .resizable(false)
            .collapsible(true)
            .default_pos([10.0, 60.0])
            .show(ctx, |ui| {
                egui::Grid::new("debug_grid").num_columns(2).show(ui, |ui| {
                    ui.label("Frame time:");
                    ui.label(format!("{:.2} ms", self.last_frame_ms));
                    ui.end_row();

                    ui.label("Tree render:");
                    ui.label(format!("{} µs", self.last_tree_render_us));
                    ui.end_row();

                    ui.label("In-flight fetches:");
                    let color = if in_flight > 0 {
                        egui::Color32::from_rgb(255, 200, 80)
                    } else {
                        egui::Color32::from_rgb(100, 200, 100)
                    };
                    ui.label(egui::RichText::new(format!("{}", in_flight)).color(color));
                    ui.end_row();

                    ui.label("Cached paths:");
                    ui.label(format!("{}", cached_keys));
                    ui.end_row();

                    ui.label("SQLite DB size:");
                    ui.label(format_bytes(db_size_bytes));
                    ui.end_row();

                    ui.label("App memory:");
                    ui.label(format_bytes(memory_bytes));
                    ui.end_row();
                });
            });
    }

    /// Shows the debug log window with registry reads and SQLite writes.
    fn show_debug_log_window(&mut self, ctx: &egui::Context) {
        let mut open = self.show_debug_log;
        egui::Window::new("Debug Log")
            .open(&mut open)
            .resizable(true)
            .default_size([600.0, 400.0])
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Filter:");
                    if ui.selectable_label(self.debug_log_filter.is_none(), "All").clicked() {
                        self.debug_log_filter = None;
                    }
                    if ui.selectable_label(self.debug_log_filter == Some(DebugCategory::RegistryRead), "Reg Read").clicked() {
                        self.debug_log_filter = Some(DebugCategory::RegistryRead);
                    }
                    if ui.selectable_label(self.debug_log_filter == Some(DebugCategory::RegistryWrite), "Reg Write").clicked() {
                        self.debug_log_filter = Some(DebugCategory::RegistryWrite);
                    }
                    if ui.selectable_label(self.debug_log_filter == Some(DebugCategory::SqliteRead), "SQL Read").clicked() {
                        self.debug_log_filter = Some(DebugCategory::SqliteRead);
                    }
                    if ui.selectable_label(self.debug_log_filter == Some(DebugCategory::SqliteWrite), "SQL Write").clicked() {
                        self.debug_log_filter = Some(DebugCategory::SqliteWrite);
                    }
                    if ui.selectable_label(self.debug_log_filter == Some(DebugCategory::Cache), "Cache").clicked() {
                        self.debug_log_filter = Some(DebugCategory::Cache);
                    }
                    
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Clear").clicked() {
                            self.store.clear_debug_log();
                        }
                    });
                });
                
                ui.separator();
                
                let events = self.store.get_debug_log();
                
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        for event in events.iter().rev().take(500) {
                            // Apply filter
                            if let Some(filter) = self.debug_log_filter {
                                if event.category != filter {
                                    continue;
                                }
                            }
                            
                            // Format timestamp as HH:MM:SS.mmm
                            let time_str = if let Ok(duration) = event.timestamp.duration_since(std::time::UNIX_EPOCH) {
                                let total_secs = duration.as_secs();
                                let hours = (total_secs / 3600) % 24;
                                let minutes = (total_secs / 60) % 60;
                                let seconds = total_secs % 60;
                                let millis = duration.subsec_millis();
                                format!("{:02}:{:02}:{:02}.{:03}", hours, minutes, seconds, millis)
                            } else {
                                "??:??:??".to_string()
                            };
                            
                            let (category_str, color) = match event.category {
                                DebugCategory::RegistryRead => ("REG_RD", egui::Color32::from_rgb(100, 180, 255)),
                                DebugCategory::RegistryWrite => ("REG_WR", egui::Color32::from_rgb(255, 150, 100)),
                                DebugCategory::SqliteRead => ("SQL_RD", egui::Color32::from_rgb(150, 255, 150)),
                                DebugCategory::SqliteWrite => ("SQL_WR", egui::Color32::from_rgb(255, 255, 100)),
                                DebugCategory::Cache => ("CACHE", egui::Color32::from_rgb(200, 150, 255)),
                            };
                            
                            ui.horizontal(|ui| {
                                ui.label(egui::RichText::new(&time_str).weak().monospace());
                                ui.label(egui::RichText::new(category_str).color(color).monospace());
                                ui.label(&event.message);
                            });
                        }
                        
                        if events.is_empty() {
                            ui.label("No events yet. Enable debug mode and interact with the registry to see events.");
                        }
                    });
            });
        self.show_debug_log = open;
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
        // Frame-time measurement
        let frame_start = Instant::now();
        if let Some(prev) = self.last_frame_start {
            self.last_frame_ms = prev.elapsed().as_secs_f32() * 1000.0;
        }
        self.last_frame_start = Some(frame_start);

        // ─────────────────────────────────────────────────────────────────
        // Search debounce check - runs every frame to ensure timer fires
        // ─────────────────────────────────────────────────────────────────
        const SEARCH_DEBOUNCE_MS: u64 = 500;
        if let Some(start) = self.search_debounce_start {
            let elapsed = start.elapsed().as_millis() as u64;
            let is_searching = self.search_state.is_searching.load(Ordering::Relaxed);
            
            if elapsed >= SEARCH_DEBOUNCE_MS 
                && !is_searching 
                && !self.search_options.query.is_empty()
                && self.search_options.query != self.last_executed_query
            {
                // Clear debounce timer and start search
                self.search_debounce_start = None;
                self.last_executed_query = self.search_options.query.clone();
                self.search_results_snapshot.clear();
                rust_hive::search::start_search_with_store(
                    self.search_options.clone(),
                    self.search_state.clone(),
                    self.store.clone(),
                    true,
                );
            } else if elapsed < SEARCH_DEBOUNCE_MS {
                // Schedule one repaint when debounce expires (not every frame)
                let remaining = std::time::Duration::from_millis(SEARCH_DEBOUNCE_MS - elapsed);
                ctx.request_repaint_after(remaining);
            }
        }

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

        // Debug overlay
        if self.show_profiler {
            self.show_debug_overlay(ctx);
        }

        // Debug log window (non-modal)
        if self.show_debug_log {
            self.show_debug_log_window(ctx);
        }

        // Keep repainting while background fetches or a full sync are in flight.
        // When a full sync completes, refresh values and check for conflicts.
        let is_syncing = self.store.is_syncing.load(Ordering::Relaxed);
        let has_pending_fetches = self.store.pending_fetches.load(Ordering::Relaxed) > 0;
        if is_syncing || has_pending_fetches {
            ctx.request_repaint();
        } else if self.was_syncing {
            // Sync just finished — refresh UI with latest data
            self.refresh_values();
            // Check for conflicts that the background push may have stored
            let conflicts = self.store.take_pending_conflicts();
            if !conflicts.is_empty() {
                self.edit_dialog = EditDialog::SyncConflicts(conflicts);
            } else {
                self.status_message = "Sync complete".to_string();
            }
        } else if self.had_pending_fetches {
            // Async value fetch just finished — refresh values panel
            self.refresh_values();
        }
        self.was_syncing = is_syncing;
        self.had_pending_fetches = has_pending_fetches;
        
        // Track search state for detecting search completion
        let is_searching = self.search_state.is_searching.load(Ordering::Relaxed);
        self.was_searching_last_frame = is_searching;
    }
}

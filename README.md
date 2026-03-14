# Hive - Windows Registry Editor

A modern, SQLite-first Windows Registry editor built with Rust and egui.

![MIT License](https://img.shields.io/badge/license-MIT-blue.svg)

## Features

- 🔍 **Browse & Edit**: Navigate the Windows Registry with a modern UI
- 💾 **SQLite-First**: Preview changes before applying to the registry
- 🔄 **Sync Control**: Pull from and push to the registry on demand
- 🔎 **Full-Text Search**: Search across keys, value names, and data
- 🔖 **Bookmarks**: Save frequently accessed keys with colors and notes
- 📁 **Import/Export**: Support for .reg file format

## Screenshots

*Coming soon*

## Installation

### From Source

```bash
git clone https://github.com/ericcdub/hive.git
cd hive
cargo build --release
```

The binary will be at `target/release/hive.exe`.

### Requirements

- Windows 10 or later
- Administrator rights (for editing protected keys)

## Architecture

Hive uses a "SQLite-first" architecture:

1. **Read**: Registry data is cached in a local SQLite database
2. **Edit**: Changes are written to SQLite first (staged)
3. **Push**: Staged changes are applied to the actual registry
4. **Pull**: Registry is re-read to sync the cache

This approach allows:
- Preview changes before applying
- Batch multiple edits together
- Detect conflicts with external changes
- Work with cached data when registry access is restricted

## Project Structure

```
hive/
├── src/
│   ├── main.rs      # Application entry point
│   └── app.rs       # Main UI (egui)
└── Cargo.toml

# rust-hive is a separate repo:
# https://github.com/ericcdub/rust-hive
```

## Rust Hive Library

The `rust-hive` crate can be used independently for Windows Registry access:

```toml
[dependencies]
rust-hive = { git = "https://github.com/ericcdub/rust-hive" }
```

```rust
use rust_hive::{SyncStore, RootKey};

let store = SyncStore::new();
store.pull_from_registry();

if let Some(values) = store.get_values(&RootKey::HkeyCurrentUser, "Environment") {
    for value in values {
        println!("{}: {}", value.name, value.data);
    }
}
```

## Keyboard Shortcuts

| Shortcut | Action |
|----------|--------|
| F5 | Refresh from registry |
| Ctrl+F | Open search |
| Ctrl+B | Open bookmarks |
| Ctrl+N | New key |
| Delete | Delete selected |
| Ctrl+C | Copy path |

## License

MIT License - see [LICENSE](LICENSE) for details.

## Author

Eric Chubb

## Contributing

Contributions are welcome! Please feel free to submit a Pull Request.

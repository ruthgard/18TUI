# 18TUI (Rust)

18TUI is a Rust rewrite of the 18xx metadata terminal UI. The goal is to reproduce the feature set of the original Ruby application while taking advantage of Rust’s safety guarantees and ecosystem.

## Status

- ✅ Workspace wiring, configuration bootstrap, and placeholder UI binary
- 🚧 Resource parsing, compatibility filtering, and ratatui interface
- 📋 Roadmap tracked in this README until an issue tracker is in place

## Requirements

- Rust toolchain (via [rustup](https://rustup.rs/))
- `cargo` (installed with Rust)
- Optional: Node.js 18+ for any JS-based tooling in `package.json`

## Quick Start

```bash
cargo check
cargo run -p tui18-tui
```

The binary ensures configuration defaults exist, syncs the engine repository checkout, and prints a stub message indicating the number of games discovered (currently zero until the loader is implemented).

## Workspace Layout

- `crates/core` – configuration, resource syncing, game metadata models, and save-file scaffolding
- `crates/tui` – binary crate hosting the ratatui-based interface shell
- `logs/` – runtime log output (ignored from source control)

## Development Notes

- `ResourceSync` shells out to `git`; consider migrating to `git2` if tighter integration or better error handling is required.
- `ResourceLoader` currently returns an empty collection; the Ruby metadata parser has not yet been ported.
- Tests live alongside the crates they target (`crates/*/tests`). Add integration smoke tests as the Rust parity grows.

## Roadmap

1. Port resource discovery (`meta.rb`) into `tui18-core::resource::loader`.
2. Mirror Ruby session models (corporations, trains, market cells) in Rust.
3. Flesh out ratatui panels, key bindings, and banner rendering.
4. Recreate save/load logic plus compatibility filtering in Rust.
5. Add automated parity tests and CI smoke runs.

## Contributing

1. Fork the repository and clone your copy.
2. Create a feature branch: `git checkout -b feature/thing`.
3. Run `cargo fmt`, `cargo clippy`, and `cargo test` before submitting PRs.
4. Open a pull request with a clear summary of changes and testing notes.

## License

This project is licensed under the [MIT License](LICENSE).

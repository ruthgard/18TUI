# 18TUI (Rust Port Scaffolding)

This workspace hosts the Rust rewrite of the 18xx metadata terminal UI. It mirrors the structure of the existing Ruby implementation while leaving space to flesh out the internals step by step.

## Workspace Layout

- `crates/core` – configuration, resource syncing, game metadata models, and save-file scaffolding.
- `crates/tui` – binary crate that will host the ratatui-based interface (currently prints a simple placeholder message).

## Getting Started

```bash
cargo check
cargo run -p tui18-tui
```

The binary presently ensures configuration defaults exist, prepares the engine repository checkout, and prints a stub message with the number of discovered games (empty until the loader is implemented).

## Roadmap (High Level)

1. Port resource discovery (`meta.rb` parsing) into `tui18-core::resource::loader`.
2. Mirror the Ruby session models (corporations, trains, market cells) in Rust data structures.
3. Implement the ratatui UI with panels, key handling, and banner rendering.
4. Recreate save/load logic and compatibility filtering in Rust.
5. Add automated tests plus smoke tests to validate parity with the Ruby version.

## Notes

- The project leans on the same crate stack as `gitparadice`/`cerTUI` (`ratatui`, `crossterm`, `tokio`, etc.).
- `ResourceSync` currently shells out to `git`; consider swapping to `git2` if tighter integration is needed.
- `ResourceLoader` returns an empty list until the parsing logic is completed.

Happy porting!

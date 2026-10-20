# PolyKara Rust core

This crate is the replacement command surface for PolyKara. It is deliberately split by responsibility:

- `config.rs` resolves `POLYKARA_HOME`, work directories, and TOML defaults.
- `manifest.rs` reads/writes `songs.csv` and adds local media.
- `lyricformats.rs` parses LRC, SRT, VTT, KRC, KSC, and plain lyric files.
- `ass.rs` emits editable ASS karaoke events.
- `verify.rs` checks lyric-source agreement.
- `project.rs` stores editable JSON projects and exports formats.
- `main.rs` owns the CLI orchestration.

Run from the repository root:

```sh
cargo test --manifest-path rust/Cargo.toml --offline
cargo run --manifest-path rust/Cargo.toml -- check
cargo run --manifest-path rust/Cargo.toml -- selftest
```

The Python implementation remains in the repository during the parity migration. GUI, web-server, provider, alignment, romanization, and Windows-packaging replacements are the next migration boundary; they are not silently removed.

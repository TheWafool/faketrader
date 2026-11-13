# FakeTrader

Rust desktop paper trader / trading trainer. eframe/egui 0.37 with the glow (OpenGL) renderer, one crate (lib + bin).

## Decisions
- Data: Yahoo v8 chart endpoint (no key), cached as JSON in the data dir (`data::data_dir()`). CSV import as fallback.
- Fills are deliberately conservative: market orders in replay fill at the next open; stops that gap fill at the open; stop beats target inside one bar (`broker.rs`).
- Pattern detectors must never look ahead. `patterns::tests::no_lookahead` guards this.
- Tunables live in `config.rs`. User-editable ones are copied into `store::Settings`.
- Brass (`theme::Pal::brass`) is only for the user's own things: orders, fills, positions, equity. Fonts: Barlow Semi Condensed (UI) and IBM Plex Mono (numbers), bundled in `assets/fonts`.
- Save file: `state.json`, written to a temp file then renamed. A file that won't parse is moved aside, never overwritten.
- Packaging: cargo-packager config in `Cargo.toml` (deb, AppImage, NSIS, WiX, app, dmg), `packaging/aur/PKGBUILD`, and GitHub Actions in `.github/workflows`.

## Status (2026-11-13)
- All six screens work against live data: Replay, Pattern quiz, Paper desk, Journal, Patterns, Settings.
- Verified locally: 14 tests, clippy clean, deb and AppImage build, PKGBUILD builds with makepkg.
- Pushed to github.com/TheWafool/faketrader; v0.1.0 released, CI and release workflow pass on Linux, Windows, macOS. Windows and macOS installers not run by hand.
- No web/wasm build. It would need ehttp in place of ureq, no threads, and browser storage, and Yahoo blocks browser CORS anyway.

## Ideas not built
- Restoring an unfinished replay session after the app restarts.
- Signed macOS and Windows builds; a Flatpak manifest.
- Multi-symbol replay; options.

## Testing the GUI
To drive the app from a script, run it under XWayland (`env -u WAYLAND_DISPLAY`) and send input with XTest via ctypes. The first click after a screenshot only focuses the window, so click twice.

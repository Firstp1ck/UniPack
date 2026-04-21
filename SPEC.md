# PackMan — Product specification

This document describes the **PackMan** TUI as implemented in this repository (see `src/lib.rs`, `src/pkg_manager.rs`, `src/package_cache.rs`, `src/all_upgradables.rs`). It is the source of truth for current behavior, not a roadmap.

## Project overview

- **Name**: PackMan
- **Kind**: Rust terminal application (`ratatui` + `crossterm`), library crate `packman` with a thin `main` binary
- **Purpose**: Browse installed packages across several backends from one UI, filter and search, install/remove/upgrade where supported, and inspect cross-manager updates in an overlay
- **Audience**: Developers and admins who use multiple package ecosystems on Linux (and macOS where Homebrew or other tools apply)

## Technology stack

- **Rust** (edition **2024**, see `Cargo.toml`)
- **ratatui** — layout, tables, tabs, styles
- **crossterm** — alternate screen, keyboard input, event polling (~100 ms)
- **tokio** — present for `spawn_blocking`-style helpers; the main loop is synchronous and uses `std::thread` for background work
- **serde** / **serde_json** — pip/JSON outputs and on-disk cache
- **regex**, **thiserror**, **chrono**, **unicode-width** — parsing, errors, and display width

## Supported package managers

Backends are **only shown if the primary executable is on `PATH`** (`command -v …`). There is no “greyed out” row for missing tools; absent binaries are omitted entirely.

| Label   | Detection command | Notes |
|---------|-------------------|--------|
| `pip`   | `pip3`            | Listing prefers JSON from `pip`/`pip3`; upgrade metadata uses outdated JSON |
| `npm`   | `npm`             | Global packages |
| `bun`   | `bun`             | Global packages |
| `cargo` | `cargo`         | `cargo install --list`; upgrade map needs `cargo-install-update` when present |
| `brew`  | `brew`          | Homebrew |
| `apt`   | `apt`           | Listing via `dpkg-query`; privileged commands use `sudo` in the shell |
| `pacman`| `pacman`        | Official repos; updates via `checkupdates` or `pacman -Qu` |
| `aur`   | `yay`           | Tab appears when **`yay`** is found; shell listing still tries `yay -Qem` then `paru -Qem` |
| `rpm`   | `rpm`           | Listing via `rpm`; Fedora-style counts/upgrades use **`dnf`** when available |
| `flatpak` | `flatpak`   | Apps column output; installs target `flathub` by default in the shell command |
| `snap`  | `snap`          | |

Privileged installs/removes/upgrades for several backends run **`sudo …`** in a subshell; there is no in-app password UI.

## User interface

### Layout (implemented)

Three vertical regions:

1. **Header** (3 lines): horizontal split — **title** (“PackMan”), **package manager tabs** (`ratatui::widgets::Tabs`, not a vertical sidebar), **distro** string from `/etc/os-release` (or simple fallbacks).
2. **Body**: either the main package `Table`, a loading line, empty-state text, or the **all upgradables** overlay table.
3. **Footer** (4 lines): multi-column key hints and a one-line **status** strip (selected package name, version, optional `→` latest, status).

Tabs can show **pending update counts** per backend when background counts have completed (e.g. `pacman (3)`).

### Main package table

- **Columns**: **Name**, **Version** (with optional inline `current → latest` and character-level diff coloring when an upgrade is known), **Status**.
- **Not shown as columns**: size and description (those fields exist on `Package` for future or parsing use but are not table columns).
- **Selection**: inverted row style for the active filtered row.
- **Scrolling**: viewport follows selection (center-biased).

### All upgradables overlay

Opened with **`a`**. Rows come from **already loaded in-memory lists** plus merged upgrade metadata — other backends only contribute after their tab has been loaded or preloaded.

- Columns: checkbox, **PM**, **Name**, **Version** (same diff styling as main table).
- **Multiselect**: Space toggles row; **`a`** select all; **`d`** deselect all; **Shift+letter** toggles rows whose PM label starts with that letter (ASCII, case-insensitive).
- **`u`**: upgrade each selected row sequentially via the correct backend.
- **Close**: `Esc` or `q`.

### Search

- **`/`** toggles **search mode** (not `Ctrl+F`).
- Filter is **case-insensitive substring** match on **package name** and **description**, not fuzzy matching.
- Allowed input in search mode: alphanumerics, `-`, `_`, `.`, `*`, plus Backspace to delete.
- **`Esc`** clears search mode and the query when leaving search.

### “Upgradable only” toggle

- **`o`** toggles `show_outdated_only`: restrict the main list to rows with a known **`latest_version`** (independent of the internal `FilterMode` enum, which defaults to **All** and has **no keybindings** in the current UI).

### Colors (Tokyo Night–style)

RGB values in code match this palette:

| Role     | Hex       |
|----------|-----------|
| Background | `#1a1b26` |
| Foreground | `#a9b1d6` |
| Primary | `#7aa2f7` |
| Secondary | `#bb9af7` |
| Accent | `#9ece6a` |
| Warning | `#e0af68` |
| Error | `#f7768e` |
| Surface | `#24283b` |
| Border | `#414868` |

Terminal UIs are **cell-based**; font size in “px” is not controlled by the application (monospace is assumed).

### Not implemented (do not assume from older drafts)

- Mouse / click / double-click handling
- Separate package **details** panel, dependency trees, or confirmation dialogs before mutating actions
- **Downgrade** operation
- **Direct keys `1`–`9`** to pick a manager (use **Tab** / **Shift+Tab** / **BackTab**)
- **`Ctrl+O`** for outdated-only (**`o`** is used instead)
- Visible **toast** for `App::message` — the field is set on success/failure but is **not rendered** in the TUI today
- Interactive column sort or status filter in the UI (`SortField` / `FilterMode` exist and affect `filtered_packages` logic but only their defaults are used without keys)

## Keyboard shortcuts (current)

| Key | Action |
|-----|--------|
| `↑` / `↓`, `k` / `j` | Move selection in the filtered list (wraps) |
| `Ctrl+d` / EOT, `Ctrl+u` / NAK | Page down/up by 20 rows (clamped) |
| `/` | Toggle search mode |
| `o` | Toggle upgradable-only vs all (main list) |
| `a` | Open all-upgradables overlay (from cached lists) |
| `u` | Upgrade selected package (main list) or upgrade selection (overlay) |
| `r` | Remove selected package |
| `i` | Install: type package name in search, then **`i`** (non-empty query required) |
| `Tab` | Next package manager |
| `Shift+Tab` / `BackTab` | Previous package manager |
| `Ctrl+R` | Refresh active list (sync) and respawn pending-update count threads |
| `Esc` | In search: leave search; otherwise quit |
| `q` | Quit (when not in search mode) |

`packman --help` / `-h` prints a short summary to stdout (no TUI).

## Behavior and data

### Listing and upgrades

- **Installed lists** run per backend in **background threads** for the active tab; other tabs may be **preloaded** (queue, max two concurrent workers) so switching tabs can reuse data.
- **Upgrade metadata** (`fetch_upgrade_versions_map`) runs per backend in threads; results merge into packages in **chunks** to keep the UI responsive.
- Subprocess calls for metadata use a **`timeout 25`** wrapper where implemented in `pkg_manager.rs`.

### Caching

- On-disk cache: **`$TMPDIR/packman/package_lists.json`** (see `package_cache.rs`).
- Invalidation uses a **fingerprint** of detected managers (name/command/list_command), not a wall-clock timestamp field.
- Load at startup is best-effort; save runs after successful loads/merges and on exit.

### Errors and edge cases

- Failed list load sets an internal message string; primary feedback is often the **empty state** or stderr-derived text on failed operations.
- Backends that fail during overlay upgrades are collected into a single summary message string.
- **No** dedicated network-error UI layer; failures surface as command errors.

## Public Rust API (crate)

Exported for tests or embedding: `run`, `App`, `App::new`, `detect_distro`, `Package`, `PackageStatus`, `FilterMode`, `SortField`, `UpgradableRow`, `collect_all_upgradables`, `collect_upgradables_from_cached_lists`, and `PackageManager` / merge helpers as exposed from `lib.rs`.

## Acceptance criteria (aligned with v0.1.0 behavior)

1. Binary runs and enters alternate-screen TUI, restores terminal on quit.
2. **`--help`** prints usage without panicking.
3. Only backends whose commands exist appear as tabs; first available tab loads in the background when possible.
4. Main table shows name, version column, and status for the active backend after load.
5. Search mode filters rows by substring (name/description).
6. **`o`** restricts to rows with known upgrade targets when metadata is present.
7. **`a`** overlay lists upgradable rows from cached data; selection and **`u`** drive upgrades.
8. **`u`** / **`r`** / **`i`** invoke the appropriate shell-backed commands for the active backend.
9. Tokyo Night–style RGB theme matches the values in code.
10. Layout uses the current terminal size from the draw surface each frame (an internal `terminal_size` field on `App` is not otherwise maintained).

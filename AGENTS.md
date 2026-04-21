# Rust development rules for AI agents (PackMan)

PackMan is a small **Rust TUI** ([ratatui](https://github.com/ratatui-org/ratatui), [crossterm](https://github.com/crossterm-rs/crossterm), [tokio](https://tokio.rs)) that lists and manages packages across **multiple backends** (pip, npm, bun, cargo, brew, apt, pacman, AUR helpers, rpm, flatpak, snap). Product intent and UI notes live in `SPEC.md`; user-facing docs in `README.md`.

## Crate layout

| Path | Role |
|------|------|
| `src/main.rs` | Thin binary entry (`packman::run()`); `#![allow(clippy::all)]` so the binary does not drive lint policy. |
| `src/lib.rs` | App state, distro/PM detection, Ratatui render loop, keyboard handling. |
| `src/pkg_manager.rs` | `PackageManager`: list/install/remove/upgrade via `std::process::Command` and shell snippets. |

New logic should live in **`lib.rs` or `pkg_manager.rs`** (or new modules under `src/`) so it is covered by normal `cargo clippy` / tests, not only behind the binary’s clippy allow.

## When creating new code

- Keep **cognitive complexity** under **25** and functions under **150 lines** (`clippy.toml`; see **Lint configuration**).
- Prefer clear **data flow**; match patterns in existing modules.
- Add `///` rustdoc for new public items; private items use `missing_docs_in_private_items` at **warn** (treated as error with `-D warnings` when that lint fires).
- For non-trivial APIs, use the **What / Inputs / Output / Details** rustdoc layout (see **Documentation**).
- Add **unit** tests for pure logic; **integration**-style checks when behavior spans `Command` / parsing boundaries (keep them hermetic where possible).

## When fixing bugs

1. Find root cause before coding.
2. Add or adjust a test that **fails** on the bug.
3. Fix; confirm the test passes; add edge cases if they prevent regressions.

## Always run after changes (repo root)

1. `cargo fmt --all`
2. `cargo clippy --all-targets --all-features -- -D warnings`
3. `cargo check`
4. `cargo test`

Use `cargo test -- --test-threads=1` if a test is sensitive to ordering (see comment in `Cargo.toml` for ignored tests).

**Optional local gate:** `./dev/scripts/security-check.sh` runs `rustfmt --check`, Clippy, and (when installed) `cargo audit`, `cargo deny check`, and `gitleaks`. There is **no** `.github/workflows` tree in this repository yet; treat the script as the closest thing to a full local CI bundle.

## Lint configuration (source of truth)

**`Cargo.toml` — `[lints.clippy]`** (excerpt; see file for full list):

```toml
[lints.clippy]
cognitive_complexity = "warn"
pedantic = { level = "warn", priority = -1 }
nursery = { level = "warn", priority = -1 }
unwrap_used = "deny"
missing_docs_in_private_items = "warn"
```

**`Cargo.toml` — `[lints.rust]`:**

```toml
[lints.rust]
missing_docs = "warn"
```

**`clippy.toml`:** `cognitive-complexity-threshold = 25`, `too-many-lines-threshold = 150`.

With `cargo clippy ... -- -D warnings`, **warnings are errors** for any lint that fires (including `cognitive_complexity`, `missing_docs`, `missing_docs_in_private_items`, and Clippy **warn** groups such as pedantic/nursery **when** a lint triggers).

## Pre-merge checklist

1. `cargo fmt --all` — clean diff.
2. `cargo clippy --all-targets --all-features -- -D warnings` — clean.
3. `cargo check` — success.
4. `cargo test` — all pass.
5. New functions respect complexity/length thresholds.
6. Documented `#[allow(...)]` only when necessary, with a short justification.

## Documentation (rustdoc)

- Public API: `///` on items that are part of the library surface.
- For non-trivial functions, prefer **What**, **Inputs**, **Output**, **Details** sections where they add clarity (all four are not required for one-liners).
- Do **not** create or edit `*.md` files (including `README.md` / `SPEC.md`) unless the user explicitly asks; prefer rustdoc.

## Code style

- **Edition:** Rust 2024 (`Cargo.toml`).
- **Errors:** Prefer `Result`; avoid `unwrap()` / `expect()` outside tests (`unwrap_used = "deny"`). Note: some startup paths in `lib.rs` still use `expect` for runtime/bootstrap; new code should not add more without discussion.
- **Control flow:** Prefer early returns over deep nesting.
- **Logging:** There is no `tracing` dependency today; do not add heavy logging without a deliberate dependency choice.

## TUI / product behavior

- **Missing tools:** Do not assume a given package manager exists. Detection is via `command -v`; unavailable backends should surface clear messages, not panic.
- **Mutating actions:** Install/remove/upgrade run real commands today—there is **no** dry-run flag. Tests must not assume a dry-run mode that does not exist.
- **Keyboard UX:** Default hints live in `render_footer()` and `run()`’s `--help` output. If default keys change, update **both** so they stay consistent.

## Shell commands and security

`src/pkg_manager.rs` builds many `sh -c` strings with **package names** and PM command names. That is sensitive territory for injection and quoting bugs.

- Prefer `Command::new(program).args([...])` **without** a shell when the invoked tool supports argv-style invocation.
- When shell one-liners are unavoidable, treat **user-controlled or list-derived names** as untrusted: validate against a conservative pattern (e.g. alphanumerics plus common package/version characters) or apply robust shell quoting **per** argument before interpolation.
- **Never** grow new `format!("... {} ...", user_input)` shell strings without an explicit safety review path.
- Long-running / network shell snippets already go through helpers like `run_shell` (timeout in `pkg_manager.rs`); keep timeouts when adding similar calls.

There are **no** application-level secrets, curl wrappers, or `PACSEA_*`-style test env vars in this repo—do not copy security checklists from other projects wholesale; anchor guidance in **this** codebase.

## Dependencies

- After adding or updating crates, run `cargo audit` (and `cargo deny check` if configured) before considering work merge-ready.
- Policy for licenses/advisories: see `deny.toml`.

## General

- Scope changes to the requested task; match existing style and module boundaries.
- Do not invent `dev/PR/`, wiki, or GitHub workflow paths that are not present unless the user asks to add that workflow.

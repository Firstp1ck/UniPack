# PackMan — AI agent quick reference

PackMan is a **Rust terminal UI** for browsing and managing packages across **pip, npm, bun, cargo, brew, apt, pacman, AUR (yay/paru), rpm, flatpak, snap**. Implementation: `src/lib.rs` (app + UI loop), `src/pkg_manager.rs` (backend commands), `src/main.rs` (binary wrapper with `#![allow(clippy::all)]`).

## Commands (run from repo root)

```bash
cargo fmt --all
cargo clippy --all-targets --all-features -- -D warnings
cargo check
cargo test
# optional full local gate (audit/deny/gitleaks if installed):
./dev/scripts/security-check.sh
```

## Rules of thumb

- **Lints:** `Cargo.toml` `[lints.*]` + `clippy.toml`; `pedantic` / `nursery` are **warn** level but still fail under `-D warnings` when they emit. `unwrap_used = "deny"`.
- **Docs:** Prefer `///` on public items; avoid unsolicited `*.md` edits.
- **PM commands:** New shell construction belongs in `pkg_manager.rs`—validate or quote package names; prefer argv-style `Command` when possible.
- **No dry-run / no tracing** in the tree today; do not document features that are not implemented.

Full detail, testing expectations, and UX notes: **`AGENTS.md`**.

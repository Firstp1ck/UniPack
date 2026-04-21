# Contributing to PackMan

Thanks for your interest in contributing. **PackMan** is a Rust terminal UI for browsing and managing packages across multiple backends (pip, npm, bun, cargo, brew, apt, pacman, AUR helpers, rpm, flatpak, snap). Product behavior is summarized in [README.md](README.md) and specified in [SPEC.md](SPEC.md).

By participating, you agree to follow our [Code of Conduct](CODE_OF_CONDUCT.md).

For a focused developer checklist (lints, tests, shell safety), see [**AGENTS.md**](AGENTS.md).

## Ways to contribute

- Bug reports and fixes
- Feature requests and implementations
- Documentation (including rustdoc) and examples
- Safer or clearer integration with a supported package manager
- UI/UX and accessibility improvements

## Before you start

- **Scope:** PackMan targets **Linux** and **macOS** where the supported tools exist; backends appear only if their executables are on `PATH`.
- **Safety:** Install, remove, and upgrade paths invoke real package-manager commands (there is **no** in-app dry-run mode today). Use a VM, container, or disposable user when exercising risky flows.
- **Security:** For suspected vulnerabilities, follow [SECURITY.md](SECURITY.md), not public issue dumps of exploit details.

## Development setup

### Prerequisites

1. **Rust** (stable, edition 2024 — see [rustup](https://rustup.rs)).
2. Clone the repository:

   ```bash
   git clone https://github.com/aliabdoxd14-sudo/packman.git
   cd packman
   ```

### Run the app

```bash
cargo run
```

### Run tests

```bash
cargo test
```

If a test is sensitive to ordering, try:

```bash
cargo test -- --test-threads=1
```

Some tests may be marked `#[ignore]`; see the note in `Cargo.toml` for how to run them when needed.

## Code quality (before you open a PR)

From the repo root:

1. **Format**

   ```bash
   cargo fmt --all
   ```

2. **Lint** (warnings are treated as errors with `-D warnings` when lints fire)

   ```bash
   cargo clippy --all-targets --all-features -- -D warnings
   ```

   Source of truth: `[lints.rust]`, `[lints.clippy]` in `Cargo.toml` and `clippy.toml`.

3. **Compile**

   ```bash
   cargo check
   ```

4. **Tests**

   ```bash
   cargo test
   ```

5. **Optional local gate** (fmt, clippy, and security tooling when installed):

   ```bash
   ./dev/scripts/security-check.sh
   ```

## Documentation

- Add `///` rustdoc for **new public** items. Private items are covered by `missing_docs_in_private_items` at **warn** (and fail under `-D warnings` when triggered).
- For non-trivial functions, a short **What / Inputs / Output / Details** layout in rustdoc is welcome when it clarifies behavior (see **AGENTS.md**).
- Avoid drive-by edits to `README.md` / `SPEC.md` unless you are explicitly updating user-facing docs as part of your change.

## Testing expectations

**Bug fixes:** add or adjust a test that fails on the bug, then fix and verify.

**New behavior:** add unit tests for pure logic; broader tests where parsing or `Command` boundaries matter. Keep tests deterministic and avoid relying on a specific machine’s global package state when you can.

## Commit and branch conventions

### Branch naming (suggested)

- `feat/<short-description>` — new features
- `fix/<short-description>` — bug fixes
- `docs/<short-description>` — documentation
- `refactor/<short-description>` — refactor without behavior change
- `chore/<short-description>` — tooling, CI, metadata

### Commit messages

[Conventional Commits](https://www.conventionalcommits.org/) are welcome, for example:

```text
fix: quote AUR package names in shell snippet

Avoids mis-parsing when names contain special characters.
```

## Pull request checklist

- [ ] `cargo fmt --all` — clean diff
- [ ] `cargo clippy --all-targets --all-features -- -D warnings` — clean
- [ ] `cargo check` — success
- [ ] `cargo test` — pass
- [ ] New/changed shell or subprocess code reviewed for injection and quoting (see **AGENTS.md** → *Shell commands and security*)
- [ ] `README.md` / `SPEC.md` updated only if user-facing behavior or defaults changed meaningfully
- [ ] Dependencies: run `cargo audit` (and `cargo deny check` if you use it) after dependency changes; do not ignore new high/critical advisories without maintainer agreement

## Security notes (short)

`src/pkg_manager.rs` builds many subprocess and shell invocations. Prefer `Command::new(…).args([…])` **without** a shell when the tool supports it. When a shell one-liner is required, treat package names and similar strings as **untrusted**: validate or quote per argument; do not add new `format!(…)` shell strings around user- or list-derived values without an explicit safety review.

Details: **AGENTS.md** and **CLAUDE.md**.

## Filing issues

### Bug reports

Include where possible:

- PackMan version (or commit hash)
- OS / distribution
- Terminal and shell
- Which package managers are relevant (and whether `sudo` is involved)
- Steps to reproduce, expected vs. actual behavior

### Feature requests

Describe the problem, proposed UX (keys, flows), and edge cases (missing tools, permissions).

Open issues here: [github.com/aliabdoxd14-sudo/packman/issues](https://github.com/aliabdoxd14-sudo/packman/issues).

## Packaging

This repository includes [`PKGBUILD-git`](PKGBUILD-git) for Arch. Packaging tweaks that only affect the AUR/git package workflow can be discussed in a PR that touches those files; broader distribution packaging may live in downstream repos.

## Code of Conduct and security

- **Code of Conduct:** [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md)
- **Security:** [SECURITY.md](SECURITY.md)

Thank you for helping improve PackMan.

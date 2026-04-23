# Implementation plan: pipx backend for UniPack

This document plans a **`pipx`** tab: list pipx-managed applications, surface upgrades where possible, and run **`pipx upgrade`** / **`pipx uninstall`** from the same patterns as other UniPack backends. Sources include the **PyPA pipx** documentation site, locally verified **`pipx --help`** output, and pipx’s published **changelog** (exit-code and JSON behaviour).

## References (official / upstream)

- **pipx home** (role, pip vs pipx, no `sudo pip install`): [pipx — Install and Run Python Applications in Isolated Environments](https://pipx.pypa.io/stable/)
- **Reference hub** (links to CLI reference, examples, environment variables): [pipx Reference](https://pipx.pypa.io/stable/reference/)
- **Changelog** (notably `pipx list --json`, exit codes): [pipx Changelog](https://pipx.pypa.io/stable/changelog/)
- **PyPI JSON API** (optional for “latest version” without spawning pip per venv): [PyPI — JSON API](https://docs.pypi.org/api/json/)

When implementing, re-run **`pipx <command> --help`** on the minimum pipx version you intend to support; flags evolve between releases.

## Goals

- Detect **`pipx`** on `PATH` and add a **`pipx`** [`PackageManager`](src/pkg_manager/mod.rs) row (same detection model as `npm` / `pip`).
- **List** installed apps with stable, parseable output.
- **Upgrade** / **uninstall** using pipx’s own commands (no direct `rm -rf` on `venvs/`).
- **Outdated / latest version** metadata: feasible but **not** a single pipx subcommand; see [Upgrade metadata strategies](#upgrade-metadata-strategies) below.
- **No elevated privileges** by default (pipx is user-local); align `needs_root` / sudo warm-up lists with **`false`** unless you explicitly support **`pipx --global`** (see [Global installs](#global-installs-pipx---global)).

## Goals *not* in scope (product parity with README)

- **Installing** new apps (`pipx install`) stays out of scope, consistent with UniPack’s “updates and removes only” stance.

## Current codebase context

| Area | Relevant today | Notes for pipx |
|------|----------------|------------------|
| Detection | [`detect.rs` `PM_CONFIGS`](src/detect.rs) | Add `("pipx", "pipx", "pipx", false)` (or tune `list_command`). |
| List / latest / counts | [`list.rs`](src/pkg_manager/list.rs), [`latest.rs`](src/pkg_manager/latest.rs), [`counts.rs`](src/pkg_manager/counts.rs) | New arms for `"pipx"`. |
| Mutations | [`commands.rs`](src/pkg_manager/commands.rs) | `pipx upgrade …`, `pipx uninstall …`; no `sudo` for default user installs. |
| Sudo warm-up | [`detect.rs` `pm_benefits_from_sudo_timestamp`](src/detect.rs), [`util.rs`](src/pkg_manager/util.rs), [`workers.rs`](src/workers.rs) | Omit **`pipx`** unless supporting `--global`. |
| Overlay / multi-upgrade | [`all_upgradables.rs`](src/all_upgradables.rs) | Ensure upgrade/remove use the **same identifier** pipx expects (venv key / package argument). |

The existing **`pip`** tab targets **libraries** (or Arch **`python-*`**) and is a different product surface than **pipx apps** (one venv per app, console scripts on `PATH`). **Do not merge** pipx into the pip tab.

## pipx behaviour that matters for UniPack

### 1. One isolated venv per application

pipx installs **applications** (packages with console entry points), each in its **own** virtual environment, then links binaries (and man pages) into the user’s **`PIPX_BIN_DIR`**. That is unlike **`pip install --user`**, which shares one environment.

**Implication:** “Package name” in the UI should match what **`pipx upgrade`** / **`pipx uninstall`** accept: the **pipx venv / package name**, not arbitrary PyPI project aliases unless they are identical.

Official overview: [pipx stable home](https://pipx.pypa.io/stable/).

### 2. Machine-readable listing: **`pipx list --json`**

`pipx list` supports **`--json`** (“rich data in json format” per **`pipx list --help`**). The document has a top-level **`pipx_spec_version`** and a **`venvs`** object whose **keys are the pipx names** you pass to other subcommands.

Each venv entry includes **`metadata.main_package`**, with useful fields such as:

- **`package`** / **`package_or_url`** — canonical package spec  
- **`package_version`** — installed version string  
- **`pinned`** — whether the app is **pinned** (upgrades may be limited until unpinned)  
- **`suffix`** — optional suffix for parallel installs  
- **`injected_packages`** — dependencies **injected** into that venv (`pipx inject`)

**Implication:** Parse JSON in Rust (serde) with a **tolerant** schema: optional fields and future **`pipx_metadata_version`** values (older/newer pipx may differ; see upstream issues around metadata version mismatches).

### 3. **`pipx list` exit code `1`**

The changelog states that **`pipx list`** can exit with **`1`** if one or more venvs need attention (metadata problems, etc.), while still being useful output in some cases.

**Implication:** Decide policy: treat **stdout JSON** as authoritative when parseable, but surface **stderr** / non-zero exit in logs or a one-line status if JSON is empty or invalid.

Source: [pipx Changelog](https://pipx.pypa.io/stable/changelog/) (search for “exit code” / “pipx list”).

### 4. **Upgrade** and **uninstall** semantics (CLI)

From **`pipx upgrade --help`** (verified locally):

- Runs **`pip install --upgrade PACKAGE`** inside the managed venv (conceptually; exact behaviour is pipx-internal).
- Supports **`--include-injected`** to upgrade injected deps too (optional UniPack flag later; v1 can omit).
- **`--force`** modifies existing venv and files under **`PIPX_BIN_DIR`** / **`PIPX_MAN_DIR`** — only if you need parity with aggressive repairs.

From **`pipx uninstall --help`**:

- **`pipx uninstall <package>`** removes that pipx-managed venv and associated symlinks.

**Identifiers:** **`pipx runpip`** takes the **venv name** as the first positional (`Name of the existing pipx-managed Virtual Environment`), matching the keys under **`venvs`** in **`pipx list --json`** (verified with **`pipx runpip --help`**).

### 5. **Pins** and **injected** packages

- **Pinned** apps: upgrading may be a no-op or require **`pipx unpin`** first; UniPack can either document this or detect `pinned: true` and show status “pinned” / disable upgrade with a clear message.
- **Injected** packages: not separate rows in default **`pipx list --json`** unless you also pass **`--include-injected`** to **`pipx list`**. Product choice: v1 lists **main apps only**; injected deps are advanced.

### 6. **Install sources other than plain PyPI names**

`package_or_url` can be a **URL**, **VCS ref**, or **local path**. **`pipx upgrade <name>`** still targets the venv by **name**; “latest” from PyPI may not apply to non-PyPI specs.

**Implication:** For “outdated” metadata, skip or special-case rows where **`package_or_url`** is not a simple PyPI name (heuristic: contains `://`, `/`, `@`, etc.), and still allow **manual upgrade** (user knows what they installed).

### 7. **Environment variables** (`PIPX_HOME`, `PIPX_BIN_DIR`, …)

pipx respects **`PIPX_HOME`**, **`PIPX_BIN_DIR`**, **`PIPX_MAN_DIR`**, **`PIPX_DEFAULT_PYTHON`**, and **`--global`** variants (see **`pipx --help`** “optional environment variables”).

**Implication:** UniPack should invoke **`pipx`** as the user’s shell would (inherit env in **`Command`**); do not hard-code `~/.local/pipx` paths. Document that custom **`PIPX_*`** layouts are supported as long as **`pipx`** on `PATH` sees them.

### 8. **Network and performance**

Any “is there a newer version?” check implies **index** or **PyPI** traffic. **`pipx list --json`** alone is **offline** for installed metadata but does **not** include “latest on PyPI” (see strategies below).

## Upgrade metadata strategies

pipx does **not** ship a single “list all outdated apps” command equivalent to **`pip list --outdated`**. Practical options:

| Strategy | Pros | Cons |
|----------|------|------|
| **A. `pipx runpip <venv> list --outdated --format=json`** | Uses the **same pip + index config** as that venv (respects private indexes, `pip.conf`, etc.). | **O(n)** subprocesses + network; slow with many venvs; must map **venv key** → subprocess. |
| **B. PyPI JSON API** `GET https://pypi.org/pypi/{name}/json` | One HTTP request per **distinct** PyPI project; faster than n pip invocations if cached. | Ignores per-venv **index-url** / credentials; wrong for private indexes unless extended. |
| **C. No metadata in `latest_map`** | Fast list-only tab; **`u`** still runs **`pipx upgrade`** (may no-op). | No **`o`** “outdated only” or counts until after a refresh strategy exists. |

**Recommendation:** Implement **A** for v1 correctness on typical setups; add **short-lived caching** (e.g. in-memory or next to existing package cache) keyed by venv path + mtime to avoid hammering the index on every Tab focus. Document **B** as an optional optimisation for public-PyPI-only users.

Verified example (no updates available → empty JSON array, exit `0`):

```bash
pipx runpip git-filter-repo list --outdated --format=json
```

## Suggested command mapping (baseline)

| Operation | Command sketch | Notes |
|-----------|----------------|-------|
| **Detect** | `command -v pipx` | Same pattern as other backends. |
| **List** | `pipx list --json` | Parse `venvs` → one `Package` per entry; **name** = venv key (upgrade/uninstall arg); **version** = `main_package.package_version`. |
| **Latest map** | Per venv: `pipx runpip <venv> list --outdated --format=json` | Parse pip’s JSON (`name`, `latest_version`); map **venv key** → latest. Skip / warn on failure per venv. |
| **Count** | Same as map length or dedupe logic | Keep consistent with merged **`PackageStatus::Outdated`**. |
| **Upgrade** | `pipx upgrade <venv>` | Match **`commands.rs`** non-interactive style; pipx does not require `-y` for upgrade in the same way apt does, but pass **`--quiet`** if you need less noise (optional). |
| **Remove** | `pipx uninstall <venv>` | Single positional package name. |

**Optional:** `pipx upgrade --force <venv>` behind a future setting if users hit broken metadata repairs.

## Code touchpoints (checklist)

1. **`src/detect.rs`** — `PM_CONFIGS` entry; **no** sudo warm-up for default pipx.
2. **`src/pkg_manager/list.rs`** — `list_pipx()` parsing **`pipx list --json`**.
3. **`src/pkg_manager/latest.rs`** — `latest_map_pipx()` using **strategy A** (or B with feature flag).
4. **`src/pkg_manager/counts.rs`** — `count_pipx_updates()`.
5. **`src/pkg_manager/commands.rs`** — `upgrade` / `remove` dispatch for **`pipx`**.
6. **`src/pkg_manager/mod.rs`** — `merge_packages_with_latest_map`: likely same as default (**exact name** key), unless you normalise case like pip.
7. **`src/all_upgradables.rs`** — tests / `upgrade_package_name`: ensure **`pipx`** uses the **venv key** from the list (should match `Package.name`).
8. **`src/ui.rs`** / **`src/run_loop.rs`** — only if pipx needs a footer note (e.g. “upgrade checks call pip per app”).
9. **Tests** — serde fixtures from real **`pipx list --json`** samples (minimal + with **`suffix`**, **`pinned`**, **`injected_packages`**); pip outdated JSON fixtures.
10. **Docs** — `README.md`, `SPEC.md`, etc., in the same PR or follow-up.

## Global installs (`pipx --global`)

`pipx` supports **`--global`** on many subcommands for multi-user installs (see **`pipx list --help`** / **`pipx --help`**).

**Product choice:**

- **v1:** User installs only — always run **`pipx list`**, **`pipx upgrade`**, **`pipx uninstall`** **without** `--global`; **`needs_root: false`**.
- **Later:** If both user and global installs matter, either a **second pseudo-backend** `pipx-global` with detection via `pipx list --global --json`, or a runtime toggle — both require **sudo** on some systems for global paths.

## QA matrix (manual)

- User with **0** pipx apps → empty tab, no crash.  
- One app **up to date** → outdated map empty; **`o`** hides row.  
- One app **outdated** (pin a lower version in a throwaway venv if needed) → row shows latest; **`u`** upgrades.  
- **Pinned** app → upgrade behaviour documented or blocked with message.  
- **Injected** venv → optional **`--include-injected`** regression if you add it.  
- **`PIPX_HOME`** overridden → list still works when env is exported in the same shell session as UniPack (inherit env).

## Open questions

- **Concurrency:** run outdated checks **in parallel** with a cap (e.g. 4) to balance speed vs PyPI rate limits.  
- **Caching:** reuse [`package_cache`](src/package_cache.rs) patterns vs a dedicated pipx-outdated TTL.  
- **Metadata-only broken venvs:** how to render rows when JSON omits **`main_package`** for an entry — skip with warning vs show “broken”.

---

*This file is an implementation plan only; authoritative behaviour after merge lives in `SPEC.md` and the code.*

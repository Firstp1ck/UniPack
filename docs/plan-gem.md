# Implementation plan: RubyGems (`gem`) backend for UniPack

This document plans a **`gem`** tab that lists **locally installed gems** for the **`gem` executable on `PATH`**, merges **outdated** metadata from **`gem outdated`**, and runs **`gem update`** / **`gem uninstall`** in line with UniPack’s non-interactive, shell-backed model ([`src/pkg_manager/`](src/pkg_manager/)).

## References (official)

Primary source: **RubyGems command reference** (same content as `gem help <command>`):

- **Command reference (single page, anchor per command):** [RubyGems Guides — Command reference](https://guides.rubygems.org/command-reference/)
- Relevant sections (anchors on that page):
  - [`gem list`](https://guides.rubygems.org/command-reference/#gem-list) — local listing, `-l` / `-a` / `-d`
  - [`gem outdated`](https://guides.rubygems.org/command-reference/#gem-outdated) — “gems you may wish to upgrade”
  - [`gem update`](https://guides.rubygems.org/command-reference/#gem-update) — update installed gems; does **not** remove old versions
  - [`gem uninstall`](https://guides.rubygems.org/command-reference/#gem-uninstall) — remove gems; dependency prompts / options
  - [`gem cleanup`](https://guides.rubygems.org/command-reference/#gem-cleanup) — optional: remove old versions after updates

RubyGems is the reference implementation; behaviour may vary slightly with Ruby packagers (distro patches), so validate on **your minimum supported Ruby / RubyGems** version.

## Goals

- Detect **`gem`** on `PATH` (typically via Ruby; often paired with **`ruby`** from rbenv/asdf/chruby or system Ruby).
- **List** installed gems with **name + version** suitable for [`Package`](src/model.rs).
- **Latest / outdated** map from **`gem outdated`** (or an equivalent documented flow).
- **Upgrade:** **`gem update <name>`** (non-interactive where the tool allows).
- **Remove:** **`gem uninstall <name>`** with flags chosen to avoid TTY prompts where possible.
- **Install** new gems stays **out of scope** (same product rule as other backends).

## How `gem` differs from pip / conda / distro package managers

### 1. **Tied to one Ruby interpreter**

The `gem` on `PATH` is **per Ruby**. Switching Ruby (rbenv, asdf, `/usr/bin/ruby` vs Homebrew Ruby) switches **GEM_HOME**, installed set, and permissions.

**Implication:** The tab represents **“gems for this `gem` binary only”**; document that users who use multiple Rubies see **one** tab per how they launched UniPack (PATH). Multi-Ruby selection is a possible **v2** (env var listing `GEM_HOME` or explicit `RUBY_ROOT`).

### 2. **User vs system install (`GEM_HOME`)**

Gems may live under the user’s home (`gem install --user-install` / modern default on some setups) or under a **system** directory writable only by root.

**Implication:**

- **`needs_root`:** default **`false`** if you only target user-writable **`gem env home`**; set **`true`** or use **`sudo`** when installs go to a root-owned **`GEM_HOME`** (detect by write test on `gem env home`, or follow the same pattern as other backends after probing).
- Align with [`ensure_privileges_ready`](src/pkg_manager/util.rs) / sudo warm-up only when you actually shell out to **`sudo gem …`**.

Official options mentioning user vs install dir: **`gem update`** / **`gem uninstall`** include **`--[no-]user-install`**, **`--install-dir`**, **`--bindir`** ([`gem update`](https://guides.rubygems.org/command-reference/#gem-update), [`gem uninstall`](https://guides.rubygems.org/command-reference/#gem-uninstall)).

### 3. **`gem list`: default is local installed set**

Official description: *“The list command is used to view the gems you have installed locally.”* Local/remote modifiers include **`-l` / `--local`**, **`-r` / `--remote`**, **`-b` / `--both`**.

**Implication:** Use **`gem list -l`** explicitly to avoid any `.gemrc` or version-dependent default quirks. Do **not** use remote listing for UniPack’s installed table.

Source: [`gem list`](https://guides.rubygems.org/command-reference/#gem-list).

### 4. **Multiple installed versions of the same name**

**`gem list`** supports **`-a` / `--all`** to *“Display all gem versions”* (otherwise you typically see one line per gem name for the default resolution behaviour).

**Product choice for v1:**

- **Simplest:** **`gem list -l`** **without** `-a` → one row per gem **name** (single displayed version). Upgrades/removes target that name; **`gem update`** moves to newest per docs.
- **Advanced:** **`gem list -l -a`** → multiple rows per name; **`gem uninstall`** then needs **`-v VERSION`** (documented on [`gem uninstall`](https://guides.rubygems.org/command-reference/#gem-uninstall)) and upgrade semantics must be defined per row.

Recommendation: **v1 without `-a`**.

### 5. **`gem outdated` is human-oriented**

Official description: *“The outdated command lists gems you may wish to upgrade to a newer version.”* There is **no documented `--json`** on the reference page for **`gem outdated`**.

**Implication:** Parse **stable text lines** (RubyGems traditionally prints lines like **`name (installed < latest)`**; confirm against `gem outdated` on Ruby 3.x and pin a **regex** + fixture tests). If locale or RubyGems changes output shape, adjust parser and fixtures.

Source: [`gem outdated`](https://guides.rubygems.org/command-reference/#gem-outdated).

### 6. **`gem update` does not remove old versions**

Official description: *“The update command does not remove the previous version. Use the cleanup command to remove old versions.”*

**Implication:** After **`gem update`**, **installed disk use** can grow; optional later action **`gem cleanup <name>`** (not required for v1). Document for users who care.

Source: [`gem update`](https://guides.rubygems.org/command-reference/#gem-update); [`gem cleanup`](https://guides.rubygems.org/command-reference/#gem-cleanup).

### 7. **`gem uninstall` and dependencies / prompts**

Official text: *“RubyGems will ask for confirmation if you are attempting to uninstall a gem that is a dependency of an existing gem. You can use the –ignore-dependencies option to skip this check.”*

Relevant flags ([`gem uninstall`](https://guides.rubygems.org/command-reference/#gem-uninstall)) include:

- **`-I` / `--ignore-dependencies`**
- **`--abort-on-dependent`** — *“Prevent uninstalling gems that are depended on by other gems.”*
- **`-x` / `--executables`** — *“Uninstall applicable executables without confirmation”*
- **`-a` / `--all`**, **`-v` / `--version`**, **`--force`**, **`--install-dir`**, **`--user-install`**, **`--platform`**, etc.

**Implication:** UniPack must choose a **non-interactive** policy (e.g. prefer **`--abort-on-dependent`** when you want safe refusal without prompts, or **`-I`** when you accept breaking dependents—product decision). **Verify** on target OS/RubyGems that your chosen argv never blocks on stdin.

### 8. **Not Bundler / not per-project**

**`gem`** manages the **global gem repository for that Ruby**. **`bundle update`** in a project is **Bundler**, different dependency graph and lockfile.

**Implication:** Name the tab **`gem`** (or “RubyGems”), not “Ruby”; document that **project gems** are out of scope unless you later add a **`bundler`** backend.

### 9. **Sources, API keys, and air-gapped hosts**

**`gem outdated`** and **`gem update`** consult **remote** sources by default (see Local/Remote options on [`gem outdated`](https://guides.rubygems.org/command-reference/#gem-outdated) / [`gem update`](https://guides.rubygems.org/command-reference/#gem-update)). Private gem servers use **`.gemrc` / source flags** (`-s`, `--clear-sources`, etc.).

**Implication:** Inherit the user’s environment and **`~/.gemrc`**; do not hard-code `rubygems.org`. Offline behaviour: commands may fail; surface stderr like other backends.

### 10. **Default / bundled gems**

Some gems are **default gems** shipped with Ruby; uninstall/update may be restricted or surprising depending on Ruby build.

**Implication:** If **`gem uninstall`** returns a non-zero exit, show stderr; optional detection of “default gem” strings in output for a clearer footer note (later).

## Suggested command mapping (baseline)

| Operation | Command sketch | Notes |
|-----------|----------------|--------|
| **Detect** | `command -v gem` | Same pattern as [`detect.rs`](src/detect.rs). |
| **List** | `gem list -l` | Parse `name (version)` lines; skip empty / headers. |
| **Latest map** | `gem outdated` | Parse lines → map **name → latest** (and compare to installed if needed). |
| **Count** | Same as outdated map size or recount lines | Keep consistent with merged [`PackageStatus::Outdated`](src/model.rs). |
| **Upgrade** | `gem update GEMNAME` | Add **`-N` / `--no-document`** if you want faster, doc-less upgrades (see [`gem update` Install/Update Options](https://guides.rubygems.org/command-reference/#gem-update)). Consider **`--conservative`** / **`--minimal-deps`** if you want narrower dependency churn (product choice). |
| **Remove** | `gem uninstall GEMNAME …` | Use the non-interactive argv policy chosen for dependency prompts (**`--abort-on-dependent`** vs **`-I`**, plus **`-x`** if needed). |

## Code touchpoints (checklist)

1. **`src/detect.rs`** — `("gem", "gem", "gem", needs_root_tbd)`.
2. **`src/pkg_manager/list.rs`** — `list_gem()` shelling out to **`gem list -l`**, parse lines into **`Package`** (optional: **`gem list -l -d`** later for **description** field).
3. **`src/pkg_manager/latest.rs`** — `latest_map_gem()` from **`gem outdated`** text parsing + tests with captured stdout.
4. **`src/pkg_manager/counts.rs`** — `count_gem_updates()`.
5. **`src/pkg_manager/commands.rs`** — `dispatch_upgrade` / `dispatch_remove` for **`gem`**; decide on **`sudo`** wrapper only when needed.
6. **`src/pkg_manager/mod.rs`** — `merge_packages_with_latest_map`: gem names are usually **case-sensitive**; match **`gem list`** naming (likely **no** `to_ascii_lowercase` like pip unless you normalise consistently in both list and outdated parsers).
7. **`src/all_upgradables.rs`** / tests — upgrade/remove use the **exact** name from the list.
8. **`src/detect.rs` / `workers.rs` / `util.rs`** — sudo warm-up only if **`needs_root`** is true for gem.
9. **Tests** — Fixture files for **`gem list -l`** and **`gem outdated`** output (multiple versions, pre-release lines if **`--prerelease`** ever used).
10. **Docs** — `README` / `SPEC` when the feature ships.

## QA matrix (manual)

- **User `GEM_HOME`** under `$HOME`: list, outdated merge, update one gem, uninstall a leaf gem.
- **System Ruby** (if available): confirm whether **`sudo`** is required and that UniPack does not hang on prompts.
- **Gem with reverse dependencies:** uninstall should either abort with clear stderr (**`--abort-on-dependent`**) or match your chosen policy.
- **Air-gapped / no index:** outdated and update fail gracefully; list still works **`-l`**.

## Open questions

- Whether to run **`gem cleanup GEMNAME`** automatically after successful **`gem update`** (saves disk; extra surprise if users expect to keep old versions).
- Whether to expose **`gem update --system`** (updates RubyGems itself) — usually **out of scope** for a package row.
- Exact **outdated** line regex across RubyGems versions and locales (stick to **C.UTF-8** / **`LC_ALL=C`** for subprocesses if needed).

---

*Implementation plan only; authoritative behaviour after merge lives in `SPEC.md` and the code.*

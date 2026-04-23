# Implementation plan: Zypper backend for UniPack

This document plans a **`zypper`** tab for openSUSE / SUSE systems: **list installed packages**, **detect installable updates**, **upgrade** and **remove** via Zypper with **non-interactive** defaults, aligned with UniPack’s other distro backends ([`src/pkg_manager/`](src/pkg_manager/), [`src/detect.rs`](src/detect.rs)).

## References (official)

- **SUSE documentation — Zypper package manager** (syntax, global **`--non-interactive`**, scripting, dry-run, subcommands overview): [Zypper package manager](https://documentation.suse.com/smart/systems-management/html/concept-zypper/index.html)
- **openSUSE / SUSE — `zypper(8)` manual** (authoritative option text for **`search`**, **`list-updates`**, **`update`**, **`remove`**, **`refresh`**, **`--terse`**, **`--xmlout`**, exit codes): [zypper.8 — openSUSE manpages](https://manpages.opensuse.org/Tumbleweed/zypper/zypper.8.en.html)

Use **`zypper help`** / **`zypper help <command>`** on the target distribution (Leap vs Tumbleweed vs SLE) to confirm option compatibility.

## Goals

- Detect **`zypper`** on `PATH` and register a **`zypper`** [`PackageManager`](src/pkg_manager/mod.rs).
- **List** installed **RPM packages** managed in the libzypp stack (Zypper’s view), with **name + version** for the UI.
- **Outdated / upgradable** metadata aligned with what **`zypper update`** would actually install (see **`list-updates`** vs **`--all`** below).
- **Upgrade:** **`zypper update <packagename>`** (or equivalent) under **`sudo`** with **global** **`--non-interactive`**.
- **Remove:** **`zypper remove <packagename>`** with the same non-interactive policy.
- **Install** new packages remains **out of scope** (UniPack product rule).

## How Zypper differs from `dnf` / `apt` / UniPack’s `rpm` tab

### 1. **Global options precede the subcommand**

Official pattern (SUSE + manpage): **`zypper [GLOBAL_OPTIONS] SUBCOMMAND [OPTIONS] [ARGS]`**. Scripting should use the **global** **`-n` / `--non-interactive`** (*“Switches to non-interactive mode… uses default answers automatically”*), not only per-command **`--no-confirm`** (*“It’s recommended to use the `--non-interactive` global option instead”*).

**Implication:** Implement argv as **`sudo zypper --non-interactive <subcommand> …`** (and add other globals like **`--auto-agree-with-licenses`** where needed for unattended updates).

Sources: [SUSE concept: Zypper](https://documentation.suse.com/smart/systems-management/html/concept-zypper/index.html); [zypper(8) global options](https://manpages.opensuse.org/Tumbleweed/zypper/zypper.8.en.html).

### 2. **Libzypp solver: “newer exists” ≠ “will be updated”**

**`list-updates` (`lu`)** documentation states it lists **only installable** updates (no dependency problems, and respects policies such as **vendor** constraints). With **`--all`**, it lists packages for which **newer versions exist**, even if **not** installable—useful for diagnostics, misleading as a sole “upgradable” source if you promise one-click upgrades.

**`update` (`up`)** documentation: it **will not** update packages that would require **vendor change** (unless allowed) and may leave items under *“The following package updates will NOT be installed”*.

**Implication:** For UniPack’s **`latest_version` / outdated rows**, prefer the same set **`list-updates`** uses by default (installable updates), unless you deliberately show “blocked” updates in a separate UI later.

Source: [zypper(8) — list-updates, update](https://manpages.opensuse.org/Tumbleweed/zypper/zypper.8.en.html) (sections around **`list-updates`** and **`update`**).

### 3. **Package types: `package` vs `patch` vs `pattern`**

Zypper handles multiple **resolvable types** (`-t` / `--type`). Patches behave differently (e.g. **remove** notes patches are not “uninstalled” like RPMs; status follows dependencies).

**Implication:** v1 scopes the tab to **`-t package`** only (simplest parity with other “package rows” backends). **`zypper patch`** / patch stacks are a **separate** product surface if ever added.

Source: [zypper(8) — remove, list-updates `-t patch`](https://manpages.opensuse.org/Tumbleweed/zypper/zypper.8.en.html).

### 4. **`remove` removes dependents; `install !pkg` is the alternative**

Official **`remove`** text: it uninstalls selected packages **and their dependent packages**, and does **not** try to install alternatives; to keep dependents, **`zypper install !name`** is suggested.

**`--clean-deps` / `--no-clean-deps`** control automatic removal of dependencies that become unneeded.

**Implication:** Same UX caution as DNF/conda: **remove** can cascade; default **`remove`** without **`--clean-deps`** unless product wants autoremove-like behaviour.

Source: [zypper(8) — remove](https://manpages.opensuse.org/Tumbleweed/zypper/zypper.8.en.html).

### 5. **Interactive patches and licenses in automation**

**`update`** documents **`--skip-interactive`** (skip patches needing reboot, messages, license confirmation) and **`--with-interactive`** (do not skip those in non-interactive mode). **`--auto-agree-with-licenses`** auto-accepts third-party license prompts.

**Implication:** For UniPack’s unattended **`update`**, combine **`--non-interactive`** with a deliberate choice: typically **`--auto-agree-with-licenses`** for parity with headless admin scripts, and decide whether **`--with-interactive`** is ever appropriate (often **no**).

Source: [zypper(8) — update](https://documentation.suse.com/smart/systems-management/html/concept-zypper/index.html) (also install/patch examples with **`--auto-agree-with-licenses`**).

### 6. **Exit codes (informational ≠ failure)**

The manual’s **EXIT CODES** section includes, among others:

- **0** — `ZYPPER_EXIT_OK`
- **100** — `ZYPPER_EXIT_INF_UPDATE_NEEDED`
- **101** — `ZYPPER_EXIT_INF_SEC_UPDATE_NEEDED`
- **102** — `ZYPPER_EXIT_INF_REBOOT_NEEDED`
- **104** — `ZYPPER_EXIT_INF_CAP_NOT_FOUND` (e.g. **search** with no matches, unless **`--ignore-unknown`**)

**`list-updates`** explicitly references exit statuses **0, 100, and 101**.

**Implication:** Shell wrappers that treat **any** non-zero status as failure will mis-handle **100/101** (and possibly **102**). Mirror the approach used for **`dnf check-update`** in UniPack: treat documented informational codes as success when parsing output, or branch on `ExitStatus::code()`.

Source: [zypper(8) — EXIT CODES, list-updates cross-reference](https://manpages.opensuse.org/Tumbleweed/zypper/zypper.8.en.html).

### 7. **Empty search and `--ignore-unknown`**

For **`search` / `info`**, the manpage states **`--ignore-unknown`** makes zypper return **`ZYPPER_EXIT_OK`** when the query matched nothing—useful for scripts.

**Implication:** If listing uses **`search`** with a filter that might legitimately return zero rows, consider **`--ignore-unknown`** to avoid false error handling; still distinguish “no packages installed” vs error.

Source: [zypper(8) — `--ignore-unknown`](https://manpages.opensuse.org/Tumbleweed/zypper/zypper.8.en.html).

### 8. **Machine-readable output: `--terse` and `--xmlout`**

Global **`--terse`**: *“Terse output for machine consumption. Implies --no-abbrev and --no-color.”*  
Global **`--xmlout`**: *“Switches to XML output… useful for scripts or graphical frontends.”*

**Implication:** Prefer **`zypper --terse`** (or **`--xmlout`** with a small XML parser) for **stable parsing** of **`search`** / **`list-updates`** instead of scraping default ASCII tables. Prototype on Tumbleweed and capture fixtures for tests.

Source: [zypper(8) — global options](https://manpages.opensuse.org/Tumbleweed/zypper/zypper.8.en.html).

### 9. **Repository refresh and offline behaviour**

Metadata refresh is central; **`zypper refresh`** exists. The manpage’s introductory material notes **`repo.refresh.delay`** in **`zypp.conf`** can delay automatic up-to-date checks so **`search`**-like operations work **without network or root** in some configurations.

**Implication:** **`refresh_mirrors_and_upgrade_package`** can map to **`sudo zypper --non-interactive refresh`** (optionally scoped) then **`update PKG`**—similar intent to pacman **`pacman -Syy`** / apt **`update`**, but follow Zypper semantics (refresh is not identical to “full dist-upgrade”).

Source: [zypper(8) — refresh, introductory repo refresh notes](https://manpages.opensuse.org/Tumbleweed/zypper/zypper.8.en.html).

### 10. **`ZYPPER_EXIT_ZYPP_LOCKED` (7) and privilege errors (5)**

Concurrent package operations (YaST, another zypper, PackageKit) can **lock** the stack.

**Implication:** Surface **“libzypp locked”** clearly in stderr; optional retry is out of scope unless product asks for it.

Source: [zypper(8) — EXIT CODES](https://manpages.opensuse.org/Tumbleweed/zypper/zypper.8.en.html).

### 11. **Transactional / immutable systems (`transactional-update`)**

SLE Micro / immutable openSUSE variants often expect **`transactional-update`** for system changes instead of calling **`zypper`** on the live root.

**Implication:** Document that the **`zypper`** tab targets **classic** mutable installs; **MicroOS**-style workflows may need a **separate** backend or detection that disables the tab with an explanation.

### 12. **Overlap with the existing `rpm` tab**

openSUSE ships **RPM** underneath; UniPack may already show an **`rpm`** tab via **`rpm -qa`**.

**Implication:** Same product discussion as in [`docs/plan-dnf.md`](docs/plan-dnf.md): **show both**, **hide `rpm` when `zypper` exists**, or document duplication—recommend **show both** for v1 unless UX feedback says otherwise.

## Suggested command mapping (baseline)

All mutating commands: **`sudo zypper --non-interactive …`** (plus license flags as chosen).

| Operation | Command sketch | Notes |
|-----------|----------------|--------|
| **Detect** | `command -v zypper` | In [`detect.rs`](src/detect.rs). |
| **List installed** | `zypper --terse search -i -t package` | **`-i` / `--installed-only`**, **`-t package`** per [zypper(8) — search](https://manpages.opensuse.org/Tumbleweed/zypper/zypper.8.en.html). Adjust if **`--terse`** output format differs by version—capture fixtures. |
| **Upgradable map** | `zypper --terse list-updates -t package` | Default **without** `--all` to match installable updates ([`list-updates` description](https://manpages.opensuse.org/Tumbleweed/zypper/zypper.8.en.html)). |
| **Count** | Parse same as map / respect exit **100/101** | Align tab badge with merged outdated rows. |
| **Upgrade** | `sudo zypper --non-interactive update -l PACKAGENAME` | Include **`-l` / `--auto-agree-with-licenses`** if third-party licenses appear in your test matrix ([zypper(8) — update](https://manpages.opensuse.org/Tumbleweed/zypper/zypper.8.en.html)). |
| **Remove** | `sudo zypper --non-interactive remove PACKAGENAME` | Avoid **`--clean-deps`** in v1 unless explicitly desired. |
| **Refresh + upgrade** | `sudo zypper --non-interactive refresh` then **`update`** | Wire into [`refresh_mirrors_and_upgrade_package`](src/pkg_manager/commands.rs) for **`zypper`**. |

## Code touchpoints (checklist)

1. **`src/detect.rs`** — `("zypper", "zypper", "zypper", true)` (or `needs_root` tied to **`sudo`** availability like other privileged PMs).
2. **`src/detect.rs`** — include **`zypper`** in `pm_benefits_from_sudo_timestamp` and help text in [`run_loop.rs`](src/run_loop.rs) if applicable.
3. **`src/pkg_manager/list.rs`** — `list_zypper()` using **`search -i -t package`** (+ **`--terse`** or **`--xmlout`**).
4. **`src/pkg_manager/latest.rs`** — `latest_map_zypper()` from **`list-updates`** (or **`update --dry-run`** if you need solver-exact diff—heavier).
5. **`src/pkg_manager/counts.rs`** — `count_zypper_updates()` with correct handling of **informational exit codes**.
6. **`src/pkg_manager/commands.rs`** — **`update`** / **`remove`**; extend **`refresh_mirrors_and_upgrade_package`** for **`zypper`** → **`refresh`**.
7. **`src/pkg_manager/util.rs`** — `ensure_privileges_ready` / sudo hints for **`zypper`**.
8. **`src/pkg_manager/mod.rs`** — **`merge_packages_with_latest_map`**: names usually match RPM **case**; likely **exact** match (not pip-style lowercasing).
9. **Tests** — Fixtures from **`zypper --terse search -i -t package`** and **`zypper --terse list-updates -t package`** on openSUSE.
10. **Docs** — `README` / `SPEC` when shipped.

## QA matrix (manual)

- **openSUSE Tumbleweed** / **Leap**: list, outdated, single **`update`**, single **`remove`** (throwaway package).
- **Vendor-stuck update**: verify a package appears in “NOT be installed” summary; UI should not claim **`latest_version`** if **`list-updates`** omits it (default policy).
- **License prompt**: confirm **`--auto-agree-with-licenses`** path in a test env if you ship it.
- **Parallel lock**: run **`zypper shell`** or another lock holder; UniPack should show failure, not hang.
- **No network**: list still useful? (depends on **`zypp.conf`** refresh delay—document behaviour.)

## Open questions

- **`--xmlout`** vs **`--terse`**: which gives the most stable column set across Leap/Tumbleweed/SLE for the same parser code.
- Whether to expose **`zypper dist-upgrade`** / **`patch`** as separate modes (almost certainly **not** v1).
- **Detect `transactional-update`** and hide or replace the **`zypper`** tab on immutable variants.

---

*Implementation plan only; authoritative behaviour after merge lives in `SPEC.md` and the code.*

# Implementation plan: DNF backend for UniPack

This document plans a first-class **`dnf`** tab in UniPack (alongside the existing **`rpm`** tab). It is informed by the upstream **DNF command reference** and Fedora / RHEL documentation, and by how UniPack already shells out for **`rpm`** (including partial DNF use today).

## References (official)

- **DNF command reference** (return codes, `list`, `check-update`, `repoquery`, `upgrade`, `remove`, global options such as `--assumeyes`): [DNF command reference](https://dnf.readthedocs.io/en/latest/command_ref.html)
- **Fedora Quick Docs — Using DNF**: [Using the DNF software package manager](https://docs.fedoraproject.org/en-US/quick-docs/dnf/)
- **RHEL — Managing software with the DNF tool** (command appendix, enterprise context): [Red Hat Documentation — DNF](https://docs.redhat.com/en/documentation/red_hat_enterprise_linux/9/html/managing_software_with_the_dnf_tool/)

## Goals

- Detect **`dnf`** on `PATH`, expose a **`dnf`** manager row, and implement **list → merge upgradables → upgrade / remove** using DNF (not raw `rpm -Uvh` / `rpm -e`) for those actions.
- Stay consistent with UniPack’s model: **non-interactive** privileged commands, **`sudo`** where required, same integration points as other backends (`detect`, `list`, `latest`, `commands`, `counts`, `util` / sudo hints, optional **refresh then upgrade** if desired).

## Current codebase context

| Area | Today (`rpm`) | Gap |
|------|----------------|-----|
| Detection | `rpm` on PATH | No row for `dnf` alone; Fedora often has both. |
| List | `rpm -qa --queryformat …` | Works but bypasses DNF’s view (repos, excludes, modular quirks). |
| Latest / upgradable map | `dnf repoquery --upgrades …` **only if** `dnf` exists (`latest_map_rpm`) | Correct high-level source, but labeled **`rpm`**. |
| Count pending | `dnf check-update` or `yum check-update` when `dnf` missing | Already DNF-aware; exit code **100** must be treated as success (see below). |
| Upgrade / remove | `sudo rpm -Uvh` / `sudo rpm -e` | **Upgrade by installed package name** is the wrong tool: `rpm -Uvh` expects package file(s) or URLs in typical usage; **DNF** is the supported way to upgrade installed NEVRAs from repos. A **`dnf`** backend should use **`dnf upgrade`** / **`dnf remove`**. |

So: adding **`dnf`** is both a **UX win** (Fedora users see the manager they actually use) and a chance to align **mutating** operations with upstream guidance.

## DNF behaviour that matters for UniPack (vs apt / pacman)

### 1. Exit code **100** on `check-update`

Official behaviour: **`dnf check-update`** exits **100** when updates are listed, **0** when none, **1** on error. That is **not** a failure for “are there updates?” workflows.

**Implication:** Any code path that treats non-zero exit as “command failed” for `check-update` (or wraps `dnf` without `; true` in a shell) must special-case **100** as success with output. UniPack’s `count_rpm_updates` already appends `; true` in a shell pipeline; a dedicated **`dnf`** count path should still document this and avoid regressions if moving to `Command` without shell.

Source: [check-update command — DNF command reference](https://dnf.readthedocs.io/en/latest/command_ref.html#check-update-command-label).

### 2. Non-interactive installs / upgrades / removes

UniPack runs without a TTY prompt for package actions. DNF expects **`--assumeyes`** / **`-y`** (documented under global **Options** in the command reference) so transactions do not block on confirmation.

Source: [DNF command reference — Options](https://dnf.readthedocs.io/en/latest/command_ref.html#options-label).

### 3. `check-update` vs `upgrade` — “available” ≠ “will install”

The reference explicitly states that a package shown by **`check-update`** may still not be installed by **`dnf upgrade`** if the solver cannot satisfy dependencies.

**Implication:** The UI may show “newer in metadata” vs “actually upgradable today”; stderr from failed **`dnf upgrade <name>`** should surface clearly (already aligned with UniPack’s error mapping pattern).

Source: same **check-update** section as above.

### 4. **Remove** pulls in dependency removal (policy)

**`dnf remove`** removes dependent packages when needed, and with default **`clean_requirements_on_remove`**, may remove dependencies that are no longer needed — stronger side effects than “remove this one row” on some other managers.

**Implication:** Behaviour matches DNF/RHEL expectations; optional later UX is a one-line warning in docs or footer for **`dnf`** only (out of scope unless product asks for it).

Source: [remove command — DNF command reference](https://dnf.readthedocs.io/en/latest/command_ref.html#remove-command-label).

### 5. **Plugins** and **command aliases**

DNF can load plugins that add commands or alter behaviour; users can define **aliases** (e.g. `rm=remove`). UniPack should invoke a **stable argv**: e.g. `dnf` and subcommands `repoquery`, `upgrade`, `remove`, not user-defined aliases.

Source: overview + **alias** command in [DNF command reference](https://dnf.readthedocs.io/en/latest/command_ref.html).

### 6. **Modularity** (RHEL 8-era streams)

Modularity is **deprecated** in current DNF docs; some systems still have modular metadata. **`dnf upgrade <name>`** can interact with module streams (see **`dnf upgrade @…`** alias in the **upgrade** section).

**Implication:** First implementation targets **plain RPM name** specs (what the list shows). Edge cases (module-only names) can be follow-up.

Source: [upgrade command — DNF command reference](https://dnf.readthedocs.io/en/latest/command_ref.html#upgrade-command-label); [module command](https://dnf.readthedocs.io/en/latest/command_ref.html#module-command-label).

### 7. **Multi-arch** and duplicate **names**

`repoquery --upgrades` may yield multiple rows per **name** (different `.arch`). UniPack’s **`Package`** model is name-centric; **`merge_packages_with_latest_map`** uses a single map key per name.

**Implication:** Document “last writer wins” or filter to `%{arch}` matching installed arch — optional refinement after baseline works on **x86_64**-only VMs.

### 8. **`microdnf`** / minimal images

Some containers ship **`microdnf`** without full **`dnf`**. Detection could be extended later (`microdnf` as alternate `command` with a narrowed subcommand set per its man page); initial plan can scope to **`dnf`** only.

### 9. **DNF 5** / distribution versions

CLI flags evolve. Prefer commands documented in the current **DNF command reference**; validate on **Fedora 40+** (or your target matrix) in CI or manual QA.

## Relationship between **`rpm`** and **`dnf`** tabs

**Options** (pick one for v1; the others are backlog):

1. **Show both** when both exist — simplest; users may see duplicate work for the same RPMDB (acceptable if documented).
2. **Hide `rpm`** when **`dnf`** is detected — avoids duplicate **all-upgradables** rows; slightly surprises users who expect `rpm`.
3. **Keep `rpm` list-only** on DNF systems — larger behavioural change to **`rpm`** tab.

Recommendation for v1: **(1) show both**, note duplication in `README` / `SPEC`; revisit if feedback is noisy.

## Suggested shell / argv mapping (baseline)

All mutating commands should run under **`sudo`** when UniPack already does so for `rpm` / `apt` (see `pkg_manager::util` and `commands.rs` patterns).

| Operation | Suggested approach | Notes |
|-----------|-------------------|--------|
| **Detect** | `command -v dnf` | New `PM_CONFIGS` row: `("dnf", "dnf", "dnf", true)` or use `list_command` = `dnf` consistently. |
| **List installed** | `dnf repoquery --installed -q --qf '%{name}\t%{evr}\n'` | Machine-readable; respects DNF’s installed set. Alternative: keep parity with `rpm -qa` but then the tab is redundant — prefer **repoquery**. |
| **Latest map** | `dnf repoquery --upgrades -q --qf '%{name}\t%{evr}\n'` | Same as today’s `latest_map_rpm` inner command; factor shared parser to avoid drift. |
| **Count** | Parse **`dnf check-update -q`** or count **`repoquery --upgrades`** lines | If using `check-update`, treat exit **100** as success. |
| **Upgrade** | `sudo dnf upgrade -y <name>` | Official **upgrade** subcommand; `-y` matches UniPack non-interactive model. |
| **Remove** | `sudo dnf remove -y <name>` | Prefer **`remove`** (aliases include `erase` but **`remove`** is the primary name). |
| **Refresh + upgrade** (optional) | `sudo dnf makecache -y` then `sudo dnf upgrade -y <name>` | **makecache** downloads/refreshes metadata ([makecache command](https://dnf.readthedocs.io/en/latest/command_ref.html#makecache-command-label)); mirrors **apt update** + upgrade mentally for users. Wire into `refresh_mirrors_and_upgrade_package` for **`dnf`** if product wants parity with pacman. |

## Code touchpoints (checklist)

Implementation order that matches existing backend shape:

1. **`src/detect.rs`**  
   - Append `dnf` to `PM_CONFIGS`.  
   - Include **`dnf`** in `pm_benefits_from_sudo_timestamp` (and any similar match sets for sudo warm-up / hints).

2. **`src/pkg_manager/list.rs`**  
   - `list_dnf()` using **`repoquery --installed`** (or agreed command) and parse into `Package`.

3. **`src/pkg_manager/latest.rs`**  
   - `latest_map_dnf()` — can call shared helper with today’s `repoquery --upgrades` parser used by `latest_map_rpm`, or delegate to one function to prevent divergence.

4. **`src/pkg_manager/commands.rs`**  
   - `dispatch_remove` / `dispatch_upgrade` arms for **`dnf`** using **`sudo dnf … -y`**.  
   - Optionally extend **`refresh_mirrors_and_upgrade_package`** for **`dnf`** → `makecache` + `upgrade`.

5. **`src/pkg_manager/counts.rs`**  
   - `count_dnf_updates()` — correct **`check-update`** semantics **or** reuse `repoquery --upgrades` line count for consistency with the map.

6. **`src/pkg_manager/util.rs`**  
   - Any `needs_sudo` / `ensure_privileges_ready` match lists: add **`dnf`**.

7. **`src/workers.rs`** / **`src/run_loop.rs`**  
   - Any hard-coded manager name lists for hints — add **`dnf`** where `rpm` appears.

8. **Tests**  
   - Parser unit tests with **fixture stdout** from `repoquery --installed` / `--upgrades` (no need for a real Fedora in unit tests).  
   - Optional integration: VM or container with `dnf`.

9. **Docs**  
   - `README.md`, `SPEC.md`, `PKGBUILD` pkgdesc, `CLAUDE.md` / `AGENTS.md` if they enumerate backends — update **after** implementation (or same PR).

## QA matrix (manual)

- **Fedora Workstation** (Wi-Fi): list, outdated merge, single upgrade, single remove.  
- **Fedora** with stale metadata: verify **`makecache`** path if implemented.  
- **System with no updates**: `check-update` exits **0**; count and overlay stay consistent.  
- **System with updates**: `check-update` exits **100**; ensure no false error logs.  
- **Conflicting upgrade** (optional): verify user-visible stderr when solver refuses.

## Open questions

- Should **`latest_map_rpm`** on DNF systems be **disabled** or redirected to avoid **double network** hits when both tabs load? (Could share a process-level cache key by RPMDB mtime — advanced.)  
- **Exact** `repoquery` flags (`--installed`, `-q`) across RHEL 8 / 9 / Fedora — confirm one minimal set and document minimum DNF version in `SPEC.md`.

---

*This file is an implementation plan only; behaviour is authoritative in `SPEC.md` and the code after merge.*

## [0.1.3] - 2026-04-23

# UniPack v0.1.3

## Highlights

- **New backend: pnpm support.** UniPack now detects and lists globally installed `pnpm` packages, including outdated counts and upgrade paths, so Node users who prefer pnpm can manage updates in the same TUI flow as npm and bun.
- **More reliable APT upgrades.** UniPack now runs `apt update` before APT package upgrades, so Debian/Ubuntu-family upgrade actions use fresh package metadata by default.
- **Smoother pacman recovery flow.** When upgrades fail because Arch mirrors need a refresh, UniPack now offers a clearer retry path and improved key handling around confirmation so you can recover and continue with less friction.
- **Stability-focused internals.** The app and package-manager code were split into smaller focused modules, which improves maintainability and reduces risk when adding or changing backends.

## Changed behavior

- **APT now refreshes metadata before upgrade.** APT-backed upgrades consistently perform `apt update` first, then retry the package upgrade with current repository state.
- **Arch/pacman upgrade retries are more guided.** Mirror-refresh retry prompts and confirmation handling were refined; if you previously saw awkward key behavior after mirror failures, the flow is now more consistent.

## Other changes

- PKGBUILD and development quality scripts were updated as part of this release cycle.

For install options, supported package managers, and the key reference, see [README](../README.md). Maintainer automation steps (tagging/AUR/wiki helpers) are available in `dev/scripts/release.sh`.

---

## [0.1.2] - 2026-04-23

# UniPack v0.1.2

## Highlights

- **Optional sudo warm-up at startup:** on a Unix TTY, when a manager that benefits from a live sudo session is present, UniPack can ask whether to run `sudo -v` before the TUI so the password prompt happens up front with normal terminal I/O. You can still skip it and authenticate manually. The privilege hint in the UI switches between “run `sudo -v`” and “sudo is enabled” once a session is warmed. If you opt in and `sudo -v` fails, the process exits non-zero so scripts see the failure clearly.
- **Arch: pip tab follows `python-*` packages:** when `pacman` is available, the pip source lists distro `python-*` packages (repos and AUR), shows the suffix after `python-`, and runs upgrades/removes through **yay** or **paru** when installed, otherwise **`sudo pacman`**, instead of treating global Python like a generic PyPI `pip` list. This matches how Arch expects system Python libraries to be managed.
- **Paru without yay:** if only **paru** is installed, the AUR backend is registered automatically (same idea as yay-only setups).
- **More accurate counts and upgrades:** Bun’s outdated count is aligned with the same list logic as the tab UI; AUR upgrade metadata is scoped to explicitly foreign (`-Qem`) packages so tab counts and bulk upgrades stay consistent.

## Changed behavior

- **Arch (and other pacman-based systems):** the **pip** tab is no longer a plain global `pip` inventory when `pacman` is on `PATH`. Expect **`python-*`** naming and pacman/AUR helper actions instead. On other platforms, pip behavior is unchanged. Details and install examples are in the project [README](../README.md).

## Other changes

- **`install.sh`** warns when fetching **darwin-x86_64** binaries (Rosetta-era) and points you at **darwin-arm64** or building from source.
- **Docs and screenshots:** README and the main screenshot were refreshed for this release (including Arch pip semantics).

For install options, supported managers, and the key reference, see [README](../README.md). Maintainer-oriented steps (AUR, tagging, etc.) live in `dev/scripts/release.sh` if you package or ship releases.

---

## [0.1.1] - 2026-04-22

# UniPack v0.1.1

## Focus

UniPack is now an **update tool**, not an installer. Use your native package manager to install new packages; use UniPack to see and apply updates across all of them from one place. Remove is still supported for cleaning up what you already have.

## Highlights

- **Security fix — shell injection closed:** privileged actions (upgrade and remove) no longer pass package names through `sh -c`. Names are handed to each package manager as a direct argument, so shell metacharacters can never be interpreted. This removes a real injection path that previously existed when typing into the search box.
- **Install feature removed:** the `i` keybinding and in-app install flow are gone. UniPack now focuses entirely on upgrades and removals. Install new packages with your native tool (e.g. `sudo pacman -S …`, `sudo apt install …`, `pip install …`).
- **Safer privileged actions:** UniPack checks sudo readiness before privileged remove and upgrade operations, with clearer one-time guidance for apt/pacman/AUR/rpm/snap flows.
- **Better upgrade feedback:** single-package upgrades show a clearer progress strip with elapsed-time feedback.
- **Faster bulk update workflow:** the all-upgradables overlay supports search filtering and clearer empty-state handling when no matches are found.
- **Improved platform behavior:** distro detection and key handling were refined for more consistent behavior across environments.

## Breaking changes

- The `i` key no longer installs packages. If you scripted or muscle-memoried the `/`-then-`i` flow, switch to your package manager's CLI for installs.
- The `i` entry is removed from the footer hint and `unipack --help` output.

## Other changes

- Project naming and repository references were aligned around **UniPack** for consistency.
- Documentation (`README.md`, `SPEC.md`) and helper scripts were refreshed to match the update-focused scope.

---


## [0.1.0] - 2026-04-22

# UniPack v0.1.0

## What you get

UniPack is one **full-screen terminal app** where you work with packages the way you already do—only you stay in one place instead of switching between different commands for pip, npm, Flatpak, your system package manager, and the rest.

If you use several of those tools, UniPack is meant to feel like a **single front door**: pick what you care about, see what is installed, and act on it with a few keys.

## Everyday use

**See everything in one rhythm.**  
Switch between package sources with Tab (and Shift+Tab to go back). Your lists stay organized per tool, so you always know *which* manager you are looking at.

**Find things quickly.**  
Turn on search, type part of a name, and the list narrows as you go. Handy when the list is long or you only remember a fragment of the package name.

**Install without memorizing flags.**  
Search for the name you want, then install from the same screen—no need to copy-paste different install syntax for each ecosystem.

**Upgrade what you use.**  
From the main list you can upgrade the package under the cursor. When you want a bigger picture, open the **all updates** view: pending updates from every supported tool show up together so you can scan, select several, and upgrade in one pass instead of visiting each tool separately.

**Spot what needs attention.**  
You can flip the current list to show only packages that have updates available, or see the full catalog when you want to browse or clean up. Where the underlying tool reports it, you can also see **what version you have** versus **what is available**, so “is this worth upgrading now?” is easier to answer at a glance.

**Refresh when the world changed.**  
If you just ran updates outside UniPack or installed something in another terminal, refresh so the screen matches reality—without restarting the app.

**A calmer screen to stare at.**  
Clear layout, readable colors, and your Linux distro name in the header so you always know which machine you are on when you jump between SSH sessions.

## Who it is for

Anyone who regularly touches **more than one** package stack—Python globals, Node globals, desktop Flatpaks, Arch AUR helpers, distro packages, and so on—and would rather **live in one TUI** than remember ten different workflows.

## Getting UniPack

Build from source or follow the options in the project [README](../README.md)—that file also lists the supported tools and a quick key reference if you want a cheat sheet beside this release note.

---

# Changelog

All notable changes to UniPack will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

---

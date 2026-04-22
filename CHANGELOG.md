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

# UniPack v0.1.4

## Highlights

- **Fuzzy search mode in both package views.** While search is active, press `Ctrl+f` to switch between normal substring matching and fuzzy subsequence matching. This works in the main package list and in the all-upgradables overlay.
- **Clearer search results at a glance.** Matching characters are now highlighted in package names, so it is easier to see *why* an entry matched your query (especially in fuzzy mode).
- **UI rendering was modularized for reliability.** The previous monolithic UI renderer was split into focused modules (main view, overlay, progress, scroll, text, theme, version-diff), reducing maintenance risk and making future UI changes safer.

## Changed behavior

- **New search toggle:** `Ctrl+f` now toggles **normal ↔ fuzzy** matching while search mode is enabled.
- **Search feedback is more explicit:** package-name match segments are visually highlighted in both normal and fuzzy search modes.

## Other changes

- Internal planning docs were added for potential future backend/system-update work (conda, dnf, gem, pipx, zypper, and verified system-update strategy design).
- PKGBUILD metadata was updated during this release cycle.

For install options, supported package managers, and the key reference, see [README](../README.md). Maintainer automation steps (tagging/AUR/wiki helpers) are available in `dev/scripts/release.sh`.

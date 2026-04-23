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

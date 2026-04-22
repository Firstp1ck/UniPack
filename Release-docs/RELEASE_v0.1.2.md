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

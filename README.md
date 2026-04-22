# 📦 UniPack

> A unified TUI for keeping every package manager up to date.

![UniPack main TUI](images/Mainpage_v0.1.2.png)

![Rust](https://img.shields.io/badge/built_with-Rust-orange?style=flat-square&logo=rust)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue?style=flat-square)](LICENSE)
![Platform](https://img.shields.io/badge/platform-Linux%20%7C%20macOS-lightgrey?style=flat-square)

UniPack lets you browse, search, upgrade, and remove packages across **pip, npm, bun, cargo, apt, pacman, AUR, rpm, flatpak, snap, and brew** — all from one terminal UI. It is focused on **keeping your system up to date**, not bootstrapping new installs: use your package manager of choice for first-time installs, then let UniPack handle the ongoing updates. It **remembers your package lists between runs** so reopening feels quicker, and it **shows when updates are available** where the underlying tools support it.

When **`pacman`** is available (Arch and other pacman-based distros), global Python libraries belong in **`python-*`** packages (official repos and often the **AUR** under the same naming). UniPack’s **pip** tab then lists those `python-*` installs, runs upgrades/removes through **`sudo pacman`** when no AUR helper is present, and through **`yay`** or **`paru`** (whichever is installed) when you have one, so behaviour matches distro conventions instead of `pip install --user` / breaking system Python.

---

## ✨ Features

- **Finds** which supported package managers are installed
- **One list per tool** — switch with Tab / Shift+Tab
- **Live search** — filter as you type (`/`)
- **Upgrade and remove** without leaving the app (installing new packages is intentionally out of scope)
- **`o`** — show only packages with updates, or everything, for the current manager
- **`a`** — see updates from **all** managers at once (Space toggles a row, **`u`** upgrades what you selected, **`a`** / **`d`** select all or none, **Shift+letter** quickly toggles rows for managers whose name starts with that letter)
- **Distro name** in the header on Linux
- **TokyoNight**-style colors
- **Eleven sources**: pip, npm, bun, cargo, brew, apt, pacman, AUR (yay or paru), rpm, flatpak, snap

---

## 📦 Supported Package Managers

| Manager   | Platform       | Notes                         |
|-----------|----------------|-------------------------------|
| `pip`     | Linux / macOS  | Elsewhere: `pip3` / PyPI. **If `pacman` exists:** installed **`python-*`** packages (repo + AUR); the list shows the **suffix after `python-`**; upgrades use **yay/paru** if available, otherwise **`sudo pacman`** |
| `npm`     | Linux / macOS  | Global packages               |
| `bun`     | Linux / macOS  | Global packages               |
| `cargo`   | Linux / macOS  | Installed crates              |
| `brew`    | macOS / Linux  | Homebrew                      |
| `apt`     | Debian/Ubuntu  | Installed packages            |
| `pacman`  | Arch Linux     | Official repos                |
| `aur`     | Arch Linux     | AUR via yay or paru           |
| `rpm`     | Fedora/RHEL    |                               |
| `flatpak` | Linux          | Flathub apps                  |
| `snap`    | Linux          |                               |

---

## 🚀 Installation

### From source

```bash
git clone https://github.com/firstp1ck/unipack
cd unipack
cargo build --release
sudo cp target/release/unipack /usr/local/bin/
```

### Arch Linux (`makepkg`)

This repository includes [`PKGBUILD`](PKGBUILD) for building and installing with Arch’s `makepkg`. It produces the `unipack-git` package (provides `unipack`) and pulls the latest sources during the build.

```bash
git clone https://github.com/firstp1ck/unipack
cd unipack
makepkg -si
```

You need the **base-devel** group (for `makepkg`) and network access so the PKGBUILD can clone the upstream tree it builds from.

### Requirements

- **Rust** — current **stable** toolchain (install or update via [rustup](https://rustup.rs))
- Any of the package managers above that you want UniPack to control
- For managers that require root (notably `apt`, `pacman`, `aur`, `rpm`, `snap`), authenticate sudo first:

```bash
sudo -v
```

UniPack runs package actions non-interactively, so this avoids password-prompt stalls during upgrade/remove.

---

## ⌨️ Keyboard Shortcuts

| Key              | Action |
|------------------|--------|
| `↑` / `k`        | Move up (wraps) |
| `↓` / `j`        | Move down (wraps) |
| `Ctrl+d` / `Ctrl+u` | Page down / up the list |
| `/`              | Toggle search mode |
| `o`              | Toggle upgradable-only vs all packages |
| `a`              | Open **all upgradables** overlay (`Esc` / `q` to close) |
| `u`              | Upgrade selected row (main list) or **selected rows** (overlay) |
| `r`              | Remove selected package |
| `Tab` / `Shift+Tab` | Next / previous package manager |
| `Ctrl+R`         | Refresh lists and pending-update counts |
| `Esc`            | Leave search, or quit when not searching |
| `q`              | Quit (only when not in search; in search, `q` is part of the query) |

---

## 🛠 Usage

```bash
# Launch UniPack
unipack

# Show help (stdout, no TUI)
unipack --help
unipack -h
```

**To upgrade:** select a row and press `u`, or press `a` for the all-managers overlay, tick rows with `Space`, and press `u`.

**To remove:** select a row and press `r`.

**To install a new package:** use your package manager directly (for example `sudo pacman -S python-<name>`, `yay -S python-<name>` / `paru -S python-<name>` when using pacman-based repos/AUR, `sudo apt install <pkg>`, or `pip install --user <pkg>` when not using that layout). UniPack intentionally does not install new packages — it is focused on updates.

---

## 🏗 Built With

- [Ratatui](https://github.com/ratatui-org/ratatui) — TUI framework
- [Crossterm](https://github.com/crossterm-rs/crossterm) — terminal backend
- [Tokio](https://tokio.rs) — async runtime
- [Serde](https://serde.rs) — reading structured output from some tools

---

## 📄 License

[MIT License](LICENSE)

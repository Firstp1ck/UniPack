# 📦 PackMan

> A fast, unified TUI for all your package managers — built with Rust.

![PackMan main TUI](images/Mainpage_v0.1.0.png)

![Rust](https://img.shields.io/badge/built_with-Rust-orange?style=flat-square&logo=rust)
![License](https://img.shields.io/badge/license-MIT-blue?style=flat-square)
![Platform](https://img.shields.io/badge/platform-Linux%20%7C%20macOS-lightgrey?style=flat-square)

PackMan lets you browse, search, install, remove, and upgrade packages across **pip, npm, bun, cargo, apt, pacman, AUR, rpm, flatpak, snap, and brew** — all from one terminal UI. It **remembers your package lists between runs** so reopening feels quicker, and it can **show when updates are available** where the underlying tools support it. Press **`a`** anytime to open an **all-updates** view across every manager PackMan found: pick multiple rows and upgrade in one go.

---

## ✨ Features

- **Finds** which supported package managers are installed
- **One list per tool** — switch with Tab / Shift+Tab
- **Live search** — filter as you type (`/`)
- **Install, remove, and upgrade** without leaving the app
- **`o`** — show only packages with updates, or everything, for the current manager
- **`a`** — see updates from **all** managers at once (Space toggles a row, **`u`** upgrades what you selected, **`a`** / **`d`** select all or none, **Shift+letter** quickly toggles rows for managers whose name starts with that letter)
- **Distro name** in the header on Linux
- **TokyoNight**-style colors
- **Eleven sources**: pip, npm, bun, cargo, brew, apt, pacman, AUR (yay or paru), rpm, flatpak, snap

---

## 📦 Supported Package Managers

| Manager   | Platform       | Notes                         |
|-----------|----------------|-------------------------------|
| `pip`     | Linux / macOS  | Prefers `pip3` when present     |
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
git clone https://github.com/aliabdoxd14-sudo/packman
cd packman
cargo build --release
sudo cp target/release/packman /usr/local/bin/
```

### Requirements

- **Rust** — current **stable** toolchain (install or update via [rustup](https://rustup.rs))
- Any of the package managers above that you want PackMan to control

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
| `i`              | Install — type the name in search first, then `i` |
| `u`              | Upgrade selected row (main list) or **selected rows** (overlay) |
| `r`              | Remove selected package |
| `Tab` / `Shift+Tab` | Next / previous package manager |
| `Ctrl+R`         | Refresh lists and pending-update counts |
| `Esc`            | Leave search, or quit when not searching |
| `q`              | Quit |

---

## 🛠 Usage

```bash
# Launch PackMan
packman

# Show help
packman --help
```

**To install a package:** press `/`, type the name, then `i`.

---

## 🏗 Built With

- [Ratatui](https://github.com/ratatui-org/ratatui) — TUI framework
- [Crossterm](https://github.com/crossterm-rs/crossterm) — terminal backend
- [Tokio](https://tokio.rs) — async runtime
- [Serde](https://serde.rs) — reading structured output from some tools

---

## 📄 License

MIT

#!/bin/bash

set -e

REPO="firstp1ck/unipack"
INSTALL_DIR="${HOME}/.local/bin"
VERSION=$(curl -sL "https://api.github.com/repos/${REPO}/releases/latest" | grep -o '"tag_name": "[^"]*' | cut -d'"' -f4)

detect_platform() {
    case "$(uname -s)" in
        Linux*)     echo "linux" ;;
        Darwin*)    echo "darwin" ;;
        *)          echo "unknown" ;;
    esac
}

detect_arch() {
    case "$(uname -m)" in
        x86_64)     echo "x86_64" ;;
        aarch64|arm64) echo "arm64" ;;
        *)          echo "x86_64" ;;
    esac
}

install() {
    local platform
    platform=$(detect_platform)
    local arch
    arch=$(detect_arch)

    if [ "$platform" = "darwin" ] && [ "$arch" = "x86_64" ]; then
        die "darwin-x86_64 binaries are no longer published. Use an Apple Silicon Mac (darwin-arm64) or build from source."
    fi
    
    local filename="unipack-${platform}-${arch}"
    local url="https://github.com/${REPO}/releases/download/${VERSION}/${filename}"
    
    echo "Downloading UniPack ${VERSION} for ${platform}-${arch}..."
    curl -fSL "$url" -o "/tmp/unipack" || die "Failed to download UniPack"
    
    if [ ! -d "$INSTALL_DIR" ]; then
        mkdir -p "$INSTALL_DIR"
    fi
    
    mv "/tmp/unipack" "${INSTALL_DIR}/unipack"
    chmod +x "${INSTALL_DIR}/unipack"
    
    echo "Installed UniPack to ${INSTALL_DIR}/unipack"
    echo "Add '${INSTALL_DIR}' to your PATH to use unipack from anywhere"
}

die() {
    echo "Error: $1" >&2
    exit 1
}

main() {
    install
}

main "$@"

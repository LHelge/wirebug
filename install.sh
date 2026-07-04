#!/bin/sh
set -eu

REPO="LHelge/wirebug"
BINARY="wirebug"
INSTALL_DIR="${WIREBUG_INSTALL_DIR:-/usr/local/bin}"

main() {
    os="$(uname -s)"
    arch="$(uname -m)"

    case "$os" in
        Linux)  platform="linux" ;;
        Darwin) platform="macos" ;;
        *)
            echo "Unsupported OS: $os"
            echo "Falling back to: cargo install --locked wirebug"
            cargo install --locked wirebug
            return
            ;;
    esac

    case "$arch" in
        x86_64|amd64)   arch="x86_64" ;;
        arm64|aarch64)   arch="aarch64" ;;
        *)
            echo "Unsupported architecture: $arch"
            echo "Falling back to: cargo install --locked wirebug"
            cargo install --locked wirebug
            return
            ;;
    esac

    # Only macOS ships an arm64 binary; Linux is x86_64-only for now.
    if [ "$platform" = "linux" ] && [ "$arch" != "x86_64" ]; then
        echo "No prebuilt Linux binary for ${arch}."
        echo "Falling back to: cargo install --locked wirebug"
        cargo install --locked wirebug
        return
    fi

    asset="${BINARY}-${platform}-${arch}"

    # Resolve latest release tag
    if command -v curl >/dev/null 2>&1; then
        tag="$(curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" \
            | grep '"tag_name"' | head -1 | cut -d'"' -f4)"
    elif command -v wget >/dev/null 2>&1; then
        tag="$(wget -qO- "https://api.github.com/repos/${REPO}/releases/latest" \
            | grep '"tag_name"' | head -1 | cut -d'"' -f4)"
    else
        echo "Neither curl nor wget found."
        echo "Falling back to: cargo install --locked wirebug"
        cargo install --locked wirebug
        return
    fi

    if [ -z "$tag" ]; then
        echo "Could not determine latest release."
        echo "Falling back to: cargo install --locked wirebug"
        cargo install --locked wirebug
        return
    fi

    url="https://github.com/${REPO}/releases/download/${tag}/${asset}"
    echo "Downloading ${BINARY} ${tag} for ${platform}-${arch}..."

    tmpdir="$(mktemp -d)"
    trap 'rm -rf "$tmpdir"' EXIT

    if command -v curl >/dev/null 2>&1; then
        if ! curl -fsSL -o "${tmpdir}/${BINARY}" "$url"; then
            echo "Binary download failed."
            echo "Falling back to: cargo install --locked wirebug"
            cargo install --locked wirebug
            return
        fi
    else
        if ! wget -qO "${tmpdir}/${BINARY}" "$url"; then
            echo "Binary download failed."
            echo "Falling back to: cargo install --locked wirebug"
            cargo install --locked wirebug
            return
        fi
    fi

    chmod +x "${tmpdir}/${BINARY}"

    echo "Installing to ${INSTALL_DIR}/${BINARY}..."
    if [ -w "$INSTALL_DIR" ]; then
        mv "${tmpdir}/${BINARY}" "${INSTALL_DIR}/${BINARY}"
    else
        sudo mv "${tmpdir}/${BINARY}" "${INSTALL_DIR}/${BINARY}"
    fi

    echo "Installed ${BINARY} ${tag} to ${INSTALL_DIR}/${BINARY}"
}

main

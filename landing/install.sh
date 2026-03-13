#!/bin/sh
# r8r installer — https://r8r.sh
# Usage: curl -sSf https://r8r.sh/install.sh | sh
#
# Environment variables:
#   R8R_VERSION      — version to install (default: latest)
#   R8R_INSTALL_DIR  — install directory (default: ~/.r8r/bin)

set -e

REPO="qhkm/r8r"
DEFAULT_INSTALL_DIR="${HOME}/.r8r/bin"
INSTALL_DIR="${R8R_INSTALL_DIR:-$DEFAULT_INSTALL_DIR}"

main() {
    detect_platform
    resolve_version
    download_and_verify
    install_binaries
    print_success
}

detect_platform() {
    OS="$(uname -s)"
    ARCH="$(uname -m)"

    case "${OS}" in
        Linux)
            case "${ARCH}" in
                x86_64)  TARGET="x86_64-unknown-linux-musl" ;;
                aarch64) TARGET="aarch64-unknown-linux-musl" ;;
                arm64)   TARGET="aarch64-unknown-linux-musl" ;;
                *)       err "Unsupported architecture: ${ARCH}" ;;
            esac
            ;;
        Darwin)
            case "${ARCH}" in
                x86_64)  TARGET="x86_64-apple-darwin" ;;
                arm64)   TARGET="aarch64-apple-darwin" ;;
                *)       err "Unsupported architecture: ${ARCH}" ;;
            esac
            ;;
        *)
            err "Unsupported OS: ${OS}. r8r supports Linux and macOS."
            ;;
    esac

    say "Detected platform: ${OS} ${ARCH} -> ${TARGET}"
}

resolve_version() {
    if [ -n "${R8R_VERSION:-}" ]; then
        VERSION="${R8R_VERSION}"
        say "Using specified version: ${VERSION}"
        return
    fi

    say "Resolving latest version..."

    # Use redirect-based approach to avoid GitHub API rate limits
    LATEST_URL="https://github.com/${REPO}/releases/latest"

    if has curl; then
        VERSION="$(curl -sI "${LATEST_URL}" 2>/dev/null | grep -i '^location:' | sed 's/.*tag\///' | tr -d '\r\n')"
    elif has wget; then
        VERSION="$(wget --spider -S "${LATEST_URL}" 2>&1 | grep -i 'location:' | sed 's/.*tag\///' | tr -d '\r\n')"
    else
        err "curl or wget is required to download r8r"
    fi

    if [ -z "${VERSION}" ]; then
        err "Could not determine latest version. Set R8R_VERSION manually."
    fi

    say "Latest version: ${VERSION}"
}

download_and_verify() {
    ARCHIVE="r8r-${VERSION}-${TARGET}.tar.gz"
    CHECKSUMS="r8r-${VERSION}-checksums.txt"
    BASE_URL="https://github.com/${REPO}/releases/download/${VERSION}"

    R8R_TMP="$(mktemp -d)"
    trap 'rm -rf "${R8R_TMP}"' EXIT

    say "Downloading ${ARCHIVE}..."
    download "${BASE_URL}/${ARCHIVE}" "${R8R_TMP}/${ARCHIVE}"

    say "Downloading checksums..."
    download "${BASE_URL}/${CHECKSUMS}" "${R8R_TMP}/${CHECKSUMS}"

    say "Verifying checksum..."
    verify_checksum "${R8R_TMP}/${ARCHIVE}" "${R8R_TMP}/${CHECKSUMS}"
}

install_binaries() {
    mkdir -p "${INSTALL_DIR}"

    say "Installing to ${INSTALL_DIR}..."
    tar xzf "${R8R_TMP}/${ARCHIVE}" -C "${INSTALL_DIR}"
    chmod +x "${INSTALL_DIR}/r8r" "${INSTALL_DIR}/r8r-mcp"
}

print_success() {
    say ""
    say "r8r ${VERSION} installed successfully!"
    say ""
    say "  r8r:     ${INSTALL_DIR}/r8r"
    say "  r8r-mcp: ${INSTALL_DIR}/r8r-mcp"
    say ""

    # Check if install dir is in PATH
    case ":${PATH}:" in
        *":${INSTALL_DIR}:"*) ;;
        *)
            say "Add r8r to your PATH by adding this to your shell profile:"
            say ""
            say "  export PATH=\"${INSTALL_DIR}:\$PATH\""
            say ""
            say "Then restart your shell or run:"
            say ""
            say "  export PATH=\"${INSTALL_DIR}:\$PATH\""
            ;;
    esac
}

# --- Utility functions ---

download() {
    URL="$1"
    DEST="$2"

    if has curl; then
        curl -sSfL "${URL}" -o "${DEST}"
    elif has wget; then
        wget -q "${URL}" -O "${DEST}"
    else
        err "curl or wget is required to download r8r"
    fi
}

verify_checksum() {
    FILE="$1"
    CHECKSUMS_FILE="$2"
    FILENAME="$(basename "${FILE}")"

    EXPECTED="$(grep "${FILENAME}" "${CHECKSUMS_FILE}" | awk '{print $1}')"
    if [ -z "${EXPECTED}" ]; then
        err "Checksum not found for ${FILENAME}"
    fi

    if has sha256sum; then
        ACTUAL="$(sha256sum "${FILE}" | awk '{print $1}')"
    elif has shasum; then
        ACTUAL="$(shasum -a 256 "${FILE}" | awk '{print $1}')"
    else
        err "sha256sum or shasum is required for checksum verification"
    fi

    if [ "${EXPECTED}" != "${ACTUAL}" ]; then
        err "Checksum verification failed!
  Expected: ${EXPECTED}
  Got:      ${ACTUAL}"
    fi

    say "Checksum verified."
}

has() {
    command -v "$1" > /dev/null 2>&1
}

say() {
    printf '%s\n' "$1"
}

err() {
    say "error: $1" >&2
    exit 1
}

main

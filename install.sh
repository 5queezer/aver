#!/usr/bin/env bash
set -euo pipefail

BIN="aver"
VERSION="${AVER_VERSION:-latest}"
INSTALL_DIR="${AVER_INSTALL_DIR:-$HOME/.cargo/bin}"
FROM_SOURCE=0
REPO="${AVER_REPO:-}"
BASE_URL="${AVER_RELEASE_BASE_URL:-}"
VERIFY_KEY="${AVER_GPG_KEYRING:-}"

usage() {
  cat <<'USAGE'
Install Aver.

Recommended secure usage:
  1. Download this script first, do not blindly pipe it to sh.
  2. Verify the script/release signature with GPG or Cosign.
  3. Run: ./install.sh --repo OWNER/REPO

Options:
  --from-source              Build and install from this checkout with cargo install.
  --repo OWNER/REPO          Install from GitHub Releases.
  --version VERSION          Release version/tag. Default: latest.
  --install-dir DIR          Install directory. Default: ~/.cargo/bin.
  --base-url URL             Direct release base URL containing assets.
  --gpg-keyring FILE         GPG keyring used to verify .asc signatures when present.
  -h, --help                 Show help.

Environment:
  AVER_REPO, AVER_VERSION, AVER_INSTALL_DIR, AVER_RELEASE_BASE_URL, AVER_GPG_KEYRING

PATH setup:
  export PATH="$HOME/.cargo/bin:$PATH"
USAGE
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --from-source) FROM_SOURCE=1 ;;
    --repo) REPO="$2"; shift ;;
    --version) VERSION="$2"; shift ;;
    --install-dir) INSTALL_DIR="$2"; shift ;;
    --base-url) BASE_URL="$2"; shift ;;
    --gpg-keyring) VERIFY_KEY="$2"; shift ;;
    -h|--help) usage; exit 0 ;;
    *) echo "unknown option: $1" >&2; usage >&2; exit 2 ;;
  esac
  shift
done

need() {
  command -v "$1" >/dev/null 2>&1 || { echo "missing required command: $1" >&2; exit 1; }
}

ensure_path_hint() {
  case ":$PATH:" in
    *":$HOME/.cargo/bin:"*) ;;
    *)
      echo
      echo "Add Aver to your PATH:"
      echo '  export PATH="$HOME/.cargo/bin:$PATH"'
      ;;
  esac
}

install_from_source() {
  need cargo
  mkdir -p "$INSTALL_DIR"
  if [ "$INSTALL_DIR" = "$HOME/.cargo/bin" ]; then
    cargo install --path crates/aver-cli --locked --force
  else
    tmp_root="$(mktemp -d)"
    trap 'rm -rf "$tmp_root"' EXIT
    cargo install --path crates/aver-cli --locked --force --root "$tmp_root"
    install -m 0755 "$tmp_root/bin/$BIN" "$INSTALL_DIR/$BIN"
  fi
}

target_triple() {
  os="$(uname -s | tr '[:upper:]' '[:lower:]')"
  arch="$(uname -m)"
  case "$arch" in
    x86_64|amd64) arch="x86_64" ;;
    aarch64|arm64) arch="aarch64" ;;
  esac
  printf '%s-%s' "$os" "$arch"
}

resolve_base_url() {
  if [ -n "$BASE_URL" ]; then
    printf '%s' "$BASE_URL"
    return
  fi
  if [ -z "$REPO" ]; then
    echo "release install needs --repo OWNER/REPO, --base-url URL, or --from-source" >&2
    exit 2
  fi
  if [ "$VERSION" = "latest" ]; then
    printf 'https://github.com/%s/releases/latest/download' "$REPO"
  else
    printf 'https://github.com/%s/releases/download/%s' "$REPO" "$VERSION"
  fi
}

download() {
  url="$1"
  out="$2"
  if command -v curl >/dev/null 2>&1; then
    curl --fail --location --proto '=https' --tlsv1.2 --output "$out" "$url"
  elif command -v wget >/dev/null 2>&1; then
    wget -O "$out" "$url"
  else
    echo "missing curl or wget" >&2
    exit 1
  fi
}

verify_sha256() {
  archive="$1"
  sums="$2"
  need sha256sum
  archive_name="$(basename "$archive")"
  grep "  $archive_name\$\| \*$archive_name\$" "$sums" > "$sums.one" || {
    echo "checksum file does not contain $archive_name" >&2
    exit 1
  }
  (cd "$(dirname "$archive")" && sha256sum -c "$(basename "$sums.one")")
}

verify_signature_if_present() {
  file="$1"
  sig="$2"
  if [ ! -s "$sig" ]; then
    echo "No signature found for $(basename "$file"); checksum verification still passed."
    return
  fi
  if command -v cosign >/dev/null 2>&1 && [ -n "${AVER_COSIGN_IDENTITY:-}" ]; then
    cosign verify-blob --signature "$sig" --certificate-identity "$AVER_COSIGN_IDENTITY" --certificate-oidc-issuer "${AVER_COSIGN_ISSUER:-https://token.actions.githubusercontent.com}" "$file"
  elif command -v gpg >/dev/null 2>&1; then
    if [ -n "$VERIFY_KEY" ]; then
      gpg --no-default-keyring --keyring "$VERIFY_KEY" --verify "$sig" "$file"
    else
      gpg --verify "$sig" "$file"
    fi
  else
    echo "signature exists but neither cosign nor gpg is available" >&2
    exit 1
  fi
}

install_from_release() {
  need tar
  mkdir -p "$INSTALL_DIR"
  tmp="$(mktemp -d)"
  trap 'rm -rf "$tmp"' EXIT
  triple="$(target_triple)"
  asset="${BIN}-${triple}.tar.gz"
  url_base="$(resolve_base_url)"

  download "$url_base/$asset" "$tmp/$asset"
  download "$url_base/SHA256SUMS" "$tmp/SHA256SUMS"
  verify_sha256 "$tmp/$asset" "$tmp/SHA256SUMS"

  download "$url_base/$asset.asc" "$tmp/$asset.asc" >/dev/null 2>&1 || true
  download "$url_base/$asset.sig" "$tmp/$asset.sig" >/dev/null 2>&1 || true
  if [ -s "$tmp/$asset.asc" ]; then
    verify_signature_if_present "$tmp/$asset" "$tmp/$asset.asc"
  else
    verify_signature_if_present "$tmp/$asset" "$tmp/$asset.sig"
  fi

  tar -xzf "$tmp/$asset" -C "$tmp"
  install -m 0755 "$tmp/$BIN" "$INSTALL_DIR/$BIN"
}

if [ "$FROM_SOURCE" -eq 1 ]; then
  install_from_source
else
  install_from_release
fi

"$INSTALL_DIR/$BIN" --version || true
echo "Installed $BIN to $INSTALL_DIR/$BIN"
ensure_path_hint

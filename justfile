# Aver project automation
# Install just: https://github.com/casey/just

set shell := ["bash", "-uc"]

export PATH := env_var('HOME') + "/.cargo/bin:" + env_var('PATH')

# List available recipes
_default:
    @just --list

# Build every workspace crate
build:
    cargo build --workspace --locked

# Run all deterministic, offline tests
test:
    cargo test --workspace --locked

# Format the workspace
fmt:
    cargo fmt --all

# Check formatting
fmt-check:
    cargo fmt --all -- --check

# Run clippy with warnings denied
clippy:
    cargo clippy --workspace --no-deps -- -D warnings

# Run the full local gate
check: fmt-check clippy test
    ./checks.sh

# Install aver from this checkout into ~/.cargo/bin
install:
    ./install.sh --from-source

# Remove the installed aver binary
uninstall:
    rm -f "${HOME}/.cargo/bin/aver"
    @echo "Removed ${HOME}/.cargo/bin/aver"

# Build the release CLI binary
release:
    cargo build --release --locked -p aver-cli

# Build the release MCP/OAuth server binary
release-server:
    cargo build --release --locked -p aver-server

# Probe the running aver-server on /healthz (up to 10 tries, 1s apart)
health:
    #!/usr/bin/env bash
    set -euo pipefail
    url="http://127.0.0.1:${AVER_PORT:-3317}/healthz"
    for _ in $(seq 1 10); do
        if body="$(curl -fsS --max-time 2 "$url" 2>/dev/null)"; then
            echo "$body"
            exit 0
        fi
        sleep 1
    done
    echo "aver-server health probe failed" >&2
    exit 1

# Build, install to ~/.local/bin, restart the user service, and probe it
deploy-server:
    #!/usr/bin/env bash
    set -euo pipefail
    cargo build --release --locked -p aver-server
    install -Dm755 target/release/aver-server "$HOME/.local/bin/aver-server"
    systemctl --user restart aver-mcp
    just health

# Install and enable the aver-mcp systemd user unit (once per machine)
install-service:
    #!/usr/bin/env bash
    set -euo pipefail
    install -Dm644 contrib/systemd/aver-mcp.service "$HOME/.config/systemd/user/aver-mcp.service"
    systemctl --user daemon-reload
    systemctl --user enable aver-mcp
    echo "aver-mcp unit installed and enabled; run 'just deploy-server' to build, install and start it."

# Rehearse the release: build as release.yml does, install into a clean container (doc/release-procedure.md)
release-dry-run:
    scripts/release-dry-run.sh

# Create a local release tarball and checksum under target/dist
dist: release
    mkdir -p target/dist
    arch="$(uname -m)"; case "$arch" in x86_64|amd64) arch="x86_64" ;; aarch64|arm64) arch="aarch64" ;; esac; tar -C target/release -czf "target/dist/aver-$(uname -s | tr '[:upper:]' '[:lower:]')-${arch}.tar.gz" aver
    sha256sum target/dist/aver-*.tar.gz > target/dist/SHA256SUMS
    @echo "Wrote target/dist/"

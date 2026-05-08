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
    ./autoresearch.checks.sh

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

# Create a local release tarball and checksum under target/dist
dist: release
    mkdir -p target/dist
    tar -C target/release -czf target/dist/aver-$(uname -s | tr '[:upper:]' '[:lower:]')-$(uname -m).tar.gz aver
    sha256sum target/dist/aver-*.tar.gz > target/dist/SHA256SUMS
    @echo "Wrote target/dist/"

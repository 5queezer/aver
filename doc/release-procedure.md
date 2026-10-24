# Release procedure

Covers roadmap Phase 4 exit: a tagged release installs and runs on a clean machine from the release artifacts alone.

## Prerequisites

- Linux x86_64 host, Rust toolchain matching `release.yml` (1.95.0), `podman` or `docker`, network access (the container runs `apt-get`).
- `just check` passes on the commit to be tagged.
- `Cargo.toml` workspace version is the one you intend to release.

## Dry run

```bash
just release-dry-run                 # or: scripts/release-dry-run.sh [OUT_DIR]
ENGINE=docker IMAGE=ubuntu:24.04 just release-dry-run
```

`scripts/release-dry-run.sh` repeats the linux-x86_64 build and publish jobs of `.github/workflows/release.yml`: `cargo build --release --locked -p aver-cli -p aver-server`, `strip`, one tarball per binary (`aver-linux-x86_64.tar.gz`, `aver-server-linux-x86_64.tar.gz`), the `aver-mcp.service` copy and `sha256sum -- *.tar.gz aver-mcp.service > SHA256SUMS`. Artifacts land in `OUT_DIR/dist` (default `$TMPDIR/aver-release-dryrun`), outside the repo. It then runs `scripts/container-verify.sh` in a fresh `ubuntu:24.04` container (same OS family as the `ubuntu-latest` build runner) with the artifacts mounted read-only.

If you change `release.yml`, change the script to match; the script is a copy, not a shared definition.

### What the container check proves

- No Rust toolchain is present (the script aborts if `cargo` exists); only `ca-certificates curl tar coreutils systemd` plus `python3`, used solely to serve the artifact directory on `127.0.0.1:8000`.
- `install.sh --base-url http://127.0.0.1:8000 --server` downloads `SHA256SUMS` and both tarballs, verifies each against the checksum file, and installs `aver` to `~/.cargo/bin` and `aver-server` to `~/.local/bin`. (`install.sh` only allows plain http for loopback URLs; everything else stays https-only.)
- `aver --version` runs; `aver-server` links against no missing shared libraries. `aver-server` has no `--version` or `--help`: any argument starts the server, so its check is the health probe.
- `aver --memory-dir /tmp/m status` works against a fresh store.
- The unit file installs to `~/.config/systemd/user/aver-mcp.service` and its `ExecStart` (`%h/.local/bin/aver-server`) resolves to an executable. `systemd-analyze verify --user` cannot run in a container without a systemd manager ("Failed to initialize manager"), so the check actually performed is the ExecStart path check. Enabling the unit is not tested; do it on the first real host.
- `aver-server` started directly with `AVER_HOST/AVER_PORT/AVER_BASE_URL/AVER_MEMORY_DIR/AVER_AUTH_DB_PATH` answers `GET /healthz`. The first curl attempt may fail before the listener is up; the script retries.

Not covered: the other three matrix targets (linux-aarch64, darwin-x86_64, darwin-aarch64), the GitHub upload step, and signature verification (no signatures are published; `install.sh` says so and continues on checksums alone).

### Recorded output (trimmed, 2026-10-24, podman 5.8.8, `ubuntu:24.04`, Fedora host)

```
49c3b113...  aver-linux-x86_64.tar.gz
66b1b215...  aver-server-linux-x86_64.tar.gz
458059e1...  aver-mcp.service
== install.sh --server
aver-linux-x86_64.tar.gz: OK
No signature found for aver-linux-x86_64.tar.gz; checksum verification still passed.
aver-server-linux-x86_64.tar.gz: OK
No signature found for aver-server-linux-x86_64.tar.gz; checksum verification still passed.
aver 0.1.0
Installed aver to /root/.cargo/bin/aver
Installed aver-server to /root/.local/bin/aver-server
== versions
aver 0.1.0
unresolved shared libs: 0
== status
aver store: ok
== unit
ExecStart target executable: /root/.local/bin/aver-server
Failed to initialize manager: No such device or address
systemd-analyze verify: failed/unavailable
== healthz
{"status":"ok"}
```

Tarball hashes differ per run (tar embeds mtimes); only the `.service` hash is stable.

## Real release

1. Merge to `master`; confirm CI is green and the dry run passes on that commit.
2. Tag and push (only the maintainer does this): `git tag vX.Y.Z && git push origin vX.Y.Z`. The `Release` workflow also has `workflow_dispatch`, which builds and keeps assets as a workflow artifact (`release-assets`) without creating a release; use it to rehearse on GitHub.
3. The workflow builds four targets, then `publish` assembles the unit and `SHA256SUMS` and creates the release with generated notes.
4. Expected assets (10 files): `aver-{linux,darwin}-{x86_64,aarch64}.tar.gz` (4), `aver-server-{linux,darwin}-{x86_64,aarch64}.tar.gz` (4), `aver-mcp.service`, `SHA256SUMS`.

## Post-release verification

On a clean machine (or the same container recipe) install from GitHub, not a local directory:

```bash
./install.sh --repo 5queezer/aver --version vX.Y.Z --server     # checksum lines must say OK
aver --version
aver --memory-dir /tmp/m status
install -Dm644 <(curl -fsSL https://github.com/5queezer/aver/releases/download/vX.Y.Z/aver-mcp.service) ~/.config/systemd/user/aver-mcp.service
systemctl --user daemon-reload && systemctl --user enable --now aver-mcp
curl -fsS http://127.0.0.1:3317/healthz
```

Confirm `SHA256SUMS` lists all eight tarballs plus the unit. Note `install.sh` does not install the systemd unit; that step is manual (or `just install-service` from a checkout).

## Rollback

Releases are immutable by convention. To roll back, point users at the previous tag: `./install.sh --repo 5queezer/aver --version vPREV --server`, then `systemctl --user restart aver-mcp`. Stores are written by the newer binary; check the release notes for schema or log-format changes before downgrading. If the bad release should disappear, mark it as pre-release or delete it in GitHub (do not move the tag), and cut a fixed `vX.Y.Z+1`.

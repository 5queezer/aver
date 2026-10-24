#!/usr/bin/env bash
# Runs INSIDE the clean container (see release-dry-run.sh). /release holds the artifacts.
set -euo pipefail
export DEBIAN_FRONTEND=noninteractive HOME=/root
apt-get update -qq >/dev/null
apt-get install -y -qq --no-install-recommends ca-certificates curl tar coreutils python3 systemd >/dev/null
! command -v cargo >/dev/null || { echo "cargo present: container is not clean" >&2; exit 1; }

(cd /release && exec python3 -m http.server 8000 --bind 127.0.0.1 >/dev/null 2>&1) &
sleep 1

echo "== install.sh --server"
bash /install.sh --base-url http://127.0.0.1:8000 --server
export PATH="$HOME/.cargo/bin:$HOME/.local/bin:$PATH"

echo "== versions"
aver --version
# aver-server has no --version/--help: any argument still starts the server, so healthz below is its check
echo "unresolved shared libs: $(ldd "$(command -v aver-server)" | grep -c "not found" || true)"

echo "== status"
aver --memory-dir /tmp/m status

echo "== unit"
unit="$HOME/.config/systemd/user/aver-mcp.service"
install -Dm644 /release/aver-mcp.service "$unit"
exec_path="$(sed -n 's|^ExecStart=%h|/root|p' "$unit")"
[ -x "$exec_path" ] && echo "ExecStart target executable: $exec_path"
if systemd-analyze verify --user "$unit" 2>&1; then echo "systemd-analyze verify: ok"; else echo "systemd-analyze verify: failed/unavailable"; fi

echo "== healthz"
AVER_HOST=127.0.0.1 AVER_PORT=3317 AVER_BASE_URL=http://127.0.0.1:3317 \
AVER_MEMORY_DIR=/tmp/srv AVER_AUTH_DB_PATH=/tmp/srv/auth.db aver-server >/tmp/server.log 2>&1 &
for _ in $(seq 10); do
  if curl -fsS --max-time 2 http://127.0.0.1:3317/healthz; then echo; exit 0; fi
  sleep 1
done
cat /tmp/server.log >&2
exit 1

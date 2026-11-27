#!/bin/bash
# MP-07 / MP-11. No registration, credentials, Docker group or service start.
set -euo pipefail
mode=${1:---plan}
case "$mode" in --plan|--apply) ;; *) echo 'usage: setup-runner.sh [--plan|--apply] [verified-runner.tar.gz SHA256]' >&2; exit 1;; esac
runner_user=chariox-release
runner_root=/var/lib/chariox-release-runner
if [ "$mode" = --plan ]; then
  cat <<'PLAN'
MP-07 / MP-11: runner installation plan (no changes)
1. Verify operator-downloaded GitHub Linux x64 runner archive against its public SHA-256.
2. Create system user chariox-release with no sudo or host Docker-group membership.
3. Install runner below /var/lib/chariox-release-runner with owner-only state.
4. Install chariox-release-runner.service; leave disabled and stopped.
5. Operator configures rootless Docker and bounded dedicated Buildx builder.
6. Coordinator exposes an approved Cargo-slot wrapper/toolchain to this user.
7. Operator registers this runner for chariox-cloud only, labels b3,chariox-release.
8. Operator enables/starts service, then sets RELEASE_RUNNER_READY=true.
PLAN
  exit 0
fi
[ "$(id -u)" = 0 ] || { echo 'MP-07 / MP-11: installation requires root' >&2; exit 1; }
archive=${2:?verified runner archive required}; digest=${3:?public SHA-256 required}
[[ "$digest" =~ ^[0-9a-f]{64}$ ]] || exit 1
printf '%s  %s\n' "$digest" "$archive" | sha256sum --check --strict - >/dev/null
[ ! -e "$runner_root" ] || { echo 'MP-07 / MP-11: runner directory already exists; refusing overwrite' >&2; exit 1; }
[ ! -e /etc/systemd/system/chariox-release-runner.service ] || exit 1
if id "$runner_user" >/dev/null 2>&1; then
  echo 'MP-07 / MP-11: dedicated user already exists; inspect instead of reusing an unknown identity' >&2; exit 1
fi
# Refuse unsafe paths before creating users or extracting as root.
python3 - "$archive" <<'PY'
import sys, tarfile
from pathlib import PurePosixPath
with tarfile.open(sys.argv[1], 'r:gz') as archive:
    for entry in archive:
        name = PurePosixPath(entry.name)
        if name.is_absolute() or '..' in name.parts or not (entry.isfile() or entry.isdir()):
            raise SystemExit('unsafe runner archive entry')
PY
useradd --system --user-group --create-home --home-dir "$runner_root" --shell /usr/sbin/nologin "$runner_user"
chmod 0700 "$runner_root"
# The source may live below root's private home. Stage this public, verified
# archive inside the new dedicated tree instead of changing parent permissions.
install -m 0644 "$archive" "$runner_root/.runner-install.tar.gz"
runuser -u "$runner_user" -- tar --no-same-owner --no-same-permissions -xzf "$runner_root/.runner-install.tar.gz" -C "$runner_root"
rm -- "$runner_root/.runner-install.tar.gz"
runner_uid=$(id -u "$runner_user")
cat > /etc/systemd/system/chariox-release-runner.service <<UNIT
[Unit]
Description=Chariox trusted main release runner (MP-07 / MP-11)
After=network-online.target user@${runner_uid}.service
Wants=network-online.target

[Service]
User=$runner_user
Group=$runner_user
WorkingDirectory=$runner_root
Environment=HOME=$runner_root
Environment=DOCKER_HOST=unix:///run/user/$runner_uid/docker.sock
Environment=NODE_OPTIONS=--max-old-space-size=4096
Environment=TMPDIR=$runner_root/tmp
ExecStart=$runner_root/run.sh
Restart=on-failure
RestartSec=10
UMask=0077
NoNewPrivileges=true
PrivateTmp=true
ProtectSystem=strict
ProtectHome=true
ReadWritePaths=$runner_root
MemoryMax=16G
CPUQuota=400%
TasksMax=1024

[Install]
WantedBy=multi-user.target
UNIT
install -d -m 0700 -o "$runner_user" -g "$runner_user" "$runner_root/tmp"
systemctl daemon-reload
echo 'MP-07 / MP-11: installed stopped/unregistered runner; complete operator settings before starting'

#!/bin/sh
# MP-07/MP-08/MP-10/MP-11: enable only in the owned diagnostic campaign image.
# Run after signed image installation, before first bootstrap/enrollment.
set -eu
test "$(id -u)" -eq 0
test "$#" -eq 1
observer_url=$1
python3 - "$observer_url" <<'PY'
import re,sys,urllib.parse
value=sys.argv[1];url=urllib.parse.urlsplit(value)
if (url.scheme!='https' or not url.hostname or url.username or url.password
        or url.query or url.fragment or not re.fullmatch(r'https://[a-zA-Z0-9.:-]+/path1-diagnostics/[a-z0-9-]{8,80}',value)):
    raise SystemExit('MP-11 exact credential-free HTTPS observer required')
PY
tool=/usr/lib/chariox/slice-build-context/deploy/managed-kernel/runtime-diagnostics.py
test -f "$tool"
# Fail instead of changing a running guest or overwriting prior campaign config.
if systemctl is-active --quiet chariox-path1-managed-bootstrap.service; then
  echo 'MP-10 campaign diagnostics must be enabled before bootstrap' >&2
  exit 1
fi
dropin=/etc/systemd/system/chariox-path1-managed-bootstrap.service.d
test ! -e "$dropin/path1-campaign-diagnostics.conf"
test ! -e /etc/systemd/system/chariox-path1-campaign-diagnostics.service
install -d -m 0700 -o chariox -g chariox /home/chariox/.chariox/runtime-diagnostics
install -d -m 0755 "$dropin"
cat > "$dropin/path1-campaign-diagnostics.conf" <<'EOF'
# MP-07/MP-08/MP-10/MP-11: campaign observation, no lifecycle authority.
[Service]
Environment=CHARIOX_RUNTIME_DIAGNOSTICS_DIR=/home/chariox/.chariox/runtime-diagnostics
EOF
cat > /etc/systemd/system/chariox-path1-campaign-diagnostics.service <<EOF
# MP-07/MP-10/MP-11: separate cgroup survives bootstrap/update restart.
[Unit]
Description=Chariox owned Path-1 campaign diagnostic shipping
After=network-online.target
Wants=network-online.target
[Service]
Type=simple
User=chariox
Group=chariox
ExecStart=/usr/bin/python3 $tool ship --directory /home/chariox/.chariox/runtime-diagnostics --observer-url $observer_url
Restart=always
RestartSec=5s
MemoryMax=64M
CPUQuota=10%
TasksMax=16
[Install]
WantedBy=multi-user.target
EOF
chmod 0644 "$dropin/path1-campaign-diagnostics.conf" /etc/systemd/system/chariox-path1-campaign-diagnostics.service
systemctl daemon-reload
# Enable in the image, start on first real guest boot. No contact with observer here.
systemctl enable chariox-path1-campaign-diagnostics.service

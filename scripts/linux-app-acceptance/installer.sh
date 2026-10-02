#!/usr/bin/env bash
# Only for the owned b6-linux-acceptance VM and the signed TEXT fixture.
# This does not execute App code or qualify a release/runtime sandbox.
set -euo pipefail
[[ $# == 1 ]] || { echo 'usage: installer.sh OWN_QGA_SOCKET' >&2; exit 2; }
here=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
python3 "$here/guest.py" --socket "$1" --timeout 300 -- /bin/bash -s <<'GUEST'
set -euo pipefail
[[ $(hostname) == b6-linux-acceptance && $(id -u) == 0 ]]
test ! -e /etc/chariox/apps/runtime-enrollment.json
findmnt -n -o OPTIONS /mnt/b6share | tr ',' '\n' | grep -qx ro
test ! -e /opt/b6-linux-acceptance
mkdir /opt/b6-linux-acceptance
cp -a /mnt/b6share/. /opt/b6-linux-acceptance/
chmod 0755 /opt/b6-linux-acceptance /opt/b6-linux-acceptance/bin
find /opt/b6-linux-acceptance/deploy -type d -exec chmod 0755 {} +
find /opt/b6-linux-acceptance/deploy -type f -exec chmod 0644 {} +
cd /opt/b6-linux-acceptance
read -r key v1 v2 < <(python3 - <<'PY'
import json
x=json.load(open('installer-fixture.json'))
assert x['qualification']=='non-executable signed text graph; installer mechanics only'
print(x['publicKeyHex'],x['records'][0]['inventorySha256'],x['records'][1]['inventorySha256'])
PY
)
installer=$PWD/bin/chariox-app-runtime-install
install_graph() { "$installer" install --source "$PWD/installer-fixture/$1" --trusted-public-key-hex "$key" --inventory-sha256 "$2"; }
refused() {
  if "$@" > /tmp/b6-refusal.out 2>&1; then echo 'unexpected success' >&2; exit 1; fi
  cat /tmp/b6-refusal.out
}
check_revision() {
  python3 - "$1" "$2" <<'PY'
import json,sys
x=json.load(open('/etc/chariox/apps/runtime-enrollment.json'))
assert x['revision']==int(sys.argv[1]), x
assert x['inventorySha256']==sys.argv[2], x
print('PASS enrollment revision', x['revision'], x['inventorySha256'])
PY
}
echo 'SCOPE: signed text graph; no executable runtime or App acceptance'
refused runuser -u b6 -- "$installer" install --source "$PWD/installer-fixture/v1" --trusted-public-key-hex "$key" --inventory-sha256 "$v1"
grep -qx app_runtime_installer_requires_root /tmp/b6-refusal.out
test ! -e /etc/chariox/apps/runtime-enrollment.json
echo 'PASS nonroot refusal'
refused "$installer" install --source "$PWD/installer-fixture/v1" --trusted-public-key-hex "$(printf '%064d' 0)" --inventory-sha256 "$v1"
test ! -e /etc/chariox/apps/runtime-enrollment.json
echo 'PASS wrong key refusal before publication'
refused "$installer" install --source "$PWD/installer-fixture/v1" --trusted-public-key-hex "$key" --inventory-sha256 "$(printf '%064d' 0)"
test ! -e /etc/chariox/apps/runtime-enrollment.json
echo 'PASS wrong inventory refusal before publication'
# The normal local installer is the only setup path, inside this VM.
bash deploy/local-linux/install-root.sh install --user b6 --bin "$PWD/bin" --runtime "$PWD/installer-fixture/v1" --runtime-key "$key" --runtime-digest "$v1"
check_revision 1 "$v1"
helper_before=$(systemctl show chariox-app-storage -p MainPID --value)
[[ $helper_before != 0 ]]
bash deploy/local-linux/install-root.sh install --user b6 --bin "$PWD/bin" --runtime "$PWD/installer-fixture/v1" --runtime-key "$key" --runtime-digest "$v1"
check_revision 1 "$v1"
[[ $(systemctl show chariox-app-storage -p MainPID --value) == "$helper_before" ]]
echo 'PASS idempotent normal root install retains helper PID'
bash deploy/local-linux/install-root.sh install --user b6 --bin "$PWD/bin" --runtime "$PWD/installer-fixture/v2" --runtime-key "$key" --runtime-digest "$v2"
check_revision 2 "$v2"
test -d "/usr/lib/chariox/app-runtimes/$v1"
echo 'PASS upgrade retains old generation'
refused "$installer" cleanup --inventory-sha256 "$v2"
test -d "/usr/lib/chariox/app-runtimes/$v2"
check_revision 2 "$v2"
echo 'PASS active generation cleanup refused'
python3 - "$installer" "$v1" <<'PY'
import fcntl,subprocess,sys
path='/usr/lib/chariox/app-runtimes/'+sys.argv[2]+'/.runtime-lease'
with open(path,'rb') as lease:
 fcntl.flock(lease,fcntl.LOCK_SH)
 result=subprocess.run([sys.argv[1],'cleanup','--inventory-sha256',sys.argv[2]],capture_output=True,text=True)
 assert result.returncode!=0, result
 assert 'busy' in result.stderr.lower(), result.stderr
 print(result.stderr.strip())
print('PASS old generation cleanup refused with shared reader lease')
PY
"$installer" cleanup --inventory-sha256 "$v1"
test ! -e "/usr/lib/chariox/app-runtimes/$v1"
check_revision 2 "$v2"
echo 'PASS inactive generation cleanup after reader release'
python3 - "$v2" <<'PY'
import hashlib,json,os,stat,sys
root='/usr/lib/chariox/app-runtimes/'+sys.argv[1]
inventory=json.load(open(root+'/runtime-inventory.json'))
for entry in inventory['files']:
 path=root+'/'+entry['path']; metadata=os.stat(path)
 assert metadata.st_uid==metadata.st_gid==0
 assert stat.S_IMODE(metadata.st_mode)==(0o555 if entry['executable'] else 0o444)
 assert hashlib.sha256(open(path,'rb').read()).hexdigest()==entry['sha256']
print('PASS installed graph ownership, modes and digests:',len(inventory['files']),'files')
PY
rm -f /tmp/b6-refusal.out
echo 'PASS mechanics complete; enrolled text fixture remains for ordinary-user boot checks'
GUEST

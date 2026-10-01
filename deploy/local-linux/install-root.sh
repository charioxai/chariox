#!/usr/bin/env bash
# The one privileged step of a local Linux Chariox install. Everything after it
# (starting the kernel, installing, updating and running Apps) runs as the
# kernel's ordinary user through install-user.sh; no further root step is needed.
#
# install, for each --user (an ordinary account that will run its own kernel):
#   1. checks the signed App runtime in --runtime against --runtime-digest (SHA-256
#      of runtime-inventory.json), then installs it with the repository's root
#      installer, which verifies its Ed25519 signature against --runtime-key before
#      publishing /usr/lib/chariox/app-runtimes/<digest> and
#      /etc/chariox/apps/runtime-enrollment.json. The key and digest come from the
#      release authority, never from the runtime directory. A failed check changes
#      nothing.
#   2. installs /usr/libexec/chariox-app-runtime-install, the root App storage
#      helper /usr/libexec/chariox-app-storage (both from --bin) and its unit
#      deploy/managed-kernel/chariox-app-storage.service;
#   3. enrolls the user in /etc/chariox/app-storage.json: uid, gid, the delegated
#      cgroup of the systemd --user unit chariox-kernel.service
#      (.../user@UID.service/app.slice/chariox-kernel.service/apps) and the kernel
#      database ~/.chariox/state/kernel.db. Other owners are kept;
#   4. loads the AppArmor profile chariox-app-bwrap when AppArmor is enabled. It
#      lets only an installed runtime's bundled bubblewrap create user namespaces,
#      which Ubuntu 23.10 and later restrict;
#   5. enables lingering, so the user's systemd manager, which delegates the
#      kernel's cgroup, runs without a login session;
#   6. starts the helper, or restarts it when its binary, unit or enrollment
#      changed (it reads the enrollment only when it starts).
# Every change is printed as it is made, and every item already in place as
# "unchanged": a repeated install with the same inputs changes nothing and leaves
# the helper running. --dry-run prints the changes without making them.
#
# uninstall --user removes each user's enrollment, their empty App storage
# directory and the lingering this script enabled (a deleted account can be named
# by uid). It refuses while the user's kernel runs or still has App storage:
# uninstall their Apps with their data and run install-user.sh uninstall first. uninstall --all
# also removes the helper, its unit, the runtime generations and their enrollment,
# and the AppArmor profile. It refuses while any App storage exists or a runtime
# generation is in use.
set -euo pipefail
umask 022

here=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
# Tests only: a fake root under which every host path below is resolved.
R=${CHARIOX_LOCAL_INSTALL_ROOT:-}
HELPER=$R/usr/libexec/chariox-app-storage
RUNTIME_INSTALLER=$R/usr/libexec/chariox-app-runtime-install
UNIT=$R/etc/systemd/system/chariox-app-storage.service
ENROLLMENT=$R/etc/chariox/app-storage.json
RUNTIME_ENROLLMENT=$R/etc/chariox/apps/runtime-enrollment.json
RUNTIMES=$R/usr/lib/chariox/app-runtimes
STORAGE=$R/var/lib/chariox-app-storage
PROFILE=$R/etc/apparmor.d/chariox-app-bwrap
APPARMOR_PROFILES=$R/sys/kernel/security/apparmor/profiles
LINGER=$R/var/lib/systemd/linger
# Lingering this script enabled, one file per uid, so uninstall reverts only that.
STATE=$R/var/lib/chariox-local-install
RESTART_PENDING=$STATE/helper-restart-pending
SERVICE=chariox-app-storage.service

say() { printf '[chariox-install] %s\n' "$*"; }
die() { printf '[chariox-install] error: %s\n' "$*" >&2; exit 1; }
usage() {
  cat <<'EOF'
usage: install-root.sh install --user NAME [--user NAME]... --bin DIR --runtime DIR
                               --runtime-key HEX --runtime-digest HEX [--dry-run]
       install-root.sh uninstall (--user NAME|UID [--user NAME|UID]... | --all) [--dry-run]
  --bin DIR             chariox-app-storage and chariox-app-runtime-install
  --runtime DIR         the signed App runtime release
  --runtime-key HEX     its trusted Ed25519 public key, from the release authority
  --runtime-digest HEX  the SHA-256 of its runtime-inventory.json, from the release authority
EOF
}
dry_run=0
# act DESCRIPTION COMMAND...: print a change, then make it unless --dry-run. A
# failure always stops the install (set -e does not apply inside conditionals).
act() {
  local what=$1; shift
  if (( dry_run )); then say "would $what"; return 0; fi
  say "$what"
  "$@" || die "failed to $what"
}

[[ $# -ge 1 ]] || { usage >&2; exit 2; }
command=$1; shift
case "$command" in
  install|uninstall) ;;
  -h|--help|help) usage; exit 0 ;;
  *) usage >&2; exit 2 ;;
esac
users=() bin= runtime= key= digest= all=0
while [[ $# -gt 0 ]]; do
  case "$1" in
    --user) users+=("${2:?--user needs a name}"); shift 2 ;;
    --bin) bin=${2:?--bin needs a directory}; shift 2 ;;
    --runtime) runtime=${2:?--runtime needs a directory}; shift 2 ;;
    --runtime-key) key=${2:?--runtime-key needs a value}; shift 2 ;;
    --runtime-digest) digest=${2:?--runtime-digest needs a value}; shift 2 ;;
    --all) all=1; shift ;;
    --dry-run) dry_run=1; shift ;;
    *) usage >&2; die "unknown argument: $1" ;;
  esac
done
[[ "$(id -u)" == 0 ]] || die "run this as root (sudo): it is the only step that needs root"
[[ "$(uname -s)" == Linux ]] || die "this installer is for Linux"
command -v python3 >/dev/null || die "python3 is required"

# Each --user as uid, and for install its owner spec uid:gid:cgroup_root:database.
uids=() owner_specs=()
for user in "${users[@]}"; do
  if [[ "$command" == uninstall && "$user" =~ ^[0-9]+$ ]]; then uids+=("$user"); continue; fi
  # The POSIX portable set; id and getent below establish that the account exists.
  [[ "$user" =~ ^[A-Za-z0-9._][A-Za-z0-9._-]*\$?$ ]] || die "not a user name: $user"
  uid=$(id -u "$user" 2>/dev/null) || die "no such user: $user"
  [[ "$uid" != 0 ]] || die "the kernel must run as an ordinary user, not root"
  uids+=("$uid")
  [[ "$command" == install ]] || continue
  gid=$(id -g "$user" 2>/dev/null) || die "cannot determine primary group: $user"
  [[ "$gid" != 0 ]] || die "$user must have a non-root primary group"
  home=$(getent passwd "$user" | cut -d: -f6)
  [[ "$home" == /?* ]] || die "$user has no home directory"
  owner_specs+=("$uid:$gid:/sys/fs/cgroup/user.slice/user-$uid.slice/user@$uid.service/app.slice/chariox-kernel.service/apps:$home/.chariox/state/kernel.db")
done

# Persist the obligation before replacing any helper startup input.
restart_obligation() {
  (( dry_run )) && return 0
  python3 - "$RESTART_PENDING" <<'PY_MARKER'
import os, sys
path = sys.argv[1]
os.makedirs(os.path.dirname(path), mode=0o755, exist_ok=True)
fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_NOFOLLOW, 0o600)
os.fsync(fd)
os.close(fd)
fd = os.open(os.path.dirname(path), os.O_RDONLY | os.O_DIRECTORY)
os.fsync(fd)
os.close(fd)
PY_MARKER
}

# enrollment install SPEC... | enrollment remove UID...: merge into or remove from
# the helper's owners. Exit 0 when the file changed (or would), 10 when unchanged.
enrollment() {
  python3 - "$ENROLLMENT" "$dry_run" "$RESTART_PENDING" "$@" <<'PY'
import json, os, sys, tempfile
path, dry_run, marker, mode, args = sys.argv[1], sys.argv[2] == "1", sys.argv[3], sys.argv[4], sys.argv[5:]
schema = "chariox.app-storage-enrollment.v1"
current = {"schema": schema, "owners": []}
if os.path.exists(path):
    with open(path) as source:
        current = json.load(source)
    if current.get("schema") != schema:
        sys.exit(f"{path} has an unknown schema")
by_uid = {owner["uid"]: dict(owner) for owner in current["owners"]}
before = json.dumps(current, sort_keys=True)
for arg in args:
    if mode == "install":
        uid, gid, cgroup_root, database = arg.split(":", 3)
        owner = by_uid.setdefault(int(uid), {"uid": int(uid), "kernel_database_paths": []})
        owner.update(gid=int(gid), cgroup_root=cgroup_root)
        if database not in owner["kernel_database_paths"]:
            owner["kernel_database_paths"].append(database)
    elif by_uid.pop(int(arg), None) is None:
        print(f"[chariox-install] uid {arg} is not enrolled")
owners = sorted(by_uid.values(), key=lambda owner: owner["uid"])
if len(owners) > 16:
    sys.exit("the App storage helper admits at most 16 owners")
updated = {"schema": schema, "owners": owners}
if json.dumps(updated, sort_keys=True) == before:
    print(f"[chariox-install] unchanged {path}")
    sys.exit(10)
if not dry_run and mode == "install":
    os.makedirs(os.path.dirname(marker), mode=0o755, exist_ok=True)
    fd = os.open(marker, os.O_WRONLY | os.O_CREAT | os.O_NOFOLLOW, 0o600)
    os.fsync(fd)
    os.close(fd)
    fd = os.open(os.path.dirname(marker), os.O_RDONLY | os.O_DIRECTORY)
    os.fsync(fd)
    os.close(fd)
verb = "would " if dry_run else ""
if not owners:
    print(f"[chariox-install] {verb}remove {path} (no owners left)")
    if not dry_run:
        os.unlink(path)
    sys.exit(0)
print(f"[chariox-install] {verb}write {path} with {len(owners)} owner(s):")
for owner in owners:
    print(f"[chariox-install]   uid {owner['uid']} gid {owner['gid']} cgroup {owner['cgroup_root']}")
if dry_run:
    sys.exit(0)
directory = os.path.dirname(path)
os.makedirs(directory, mode=0o755, exist_ok=True)
fd, pending = tempfile.mkstemp(dir=directory, prefix=".app-storage.")
with os.fdopen(fd, "w") as out:
    json.dump(updated, out, separators=(",", ":"))
    out.write("\n")
    out.flush()
    os.fsync(out.fileno())
os.chmod(pending, 0o644)
os.rename(pending, path)
directory_fd = os.open(directory, os.O_RDONLY)
os.fsync(directory_fd)
os.close(directory_fd)
PY
}

# put SOURCE DEST MODE: install SOURCE as DEST, owned by this (root) installer,
# unless DEST already holds the same bytes and mode. Returns 1 when unchanged.
put() {
  local source=$1 dest=$2 mode=$3
  if [[ -f "$dest" && ! -L "$dest" && -O "$dest" && -G "$dest" ]] \
    && [[ "$(stat -c %a -- "$dest")" == "${mode#0}" ]] && cmp -s -- "$source" "$dest"; then
    say "unchanged $dest"
    return 1
  fi
  if [[ ${4:-} == restart ]]; then restart_obligation || die "cannot persist helper activation obligation"; fi
  act "install $dest" install -D -o root -g root -m "$mode" -- "$source" "$dest.chariox-new"
  (( dry_run )) || mv -f -- "$dest.chariox-new" "$dest" || die "failed to install $dest"
}

remove() { # PATH...: remove each that exists
  local path
  for path in "$@"; do
    if [[ -e "$path" || -L "$path" ]]; then act "remove $path" rm -f -- "$path"; fi
  done
}

enrolled_runtime() {
  [[ -f "$RUNTIME_ENROLLMENT" ]] || { echo none; return; }
  python3 -c 'import json,sys; e=json.load(open(sys.argv[1])); print(e["inventorySha256"], "revision", e["revision"])' "$RUNTIME_ENROLLMENT"
}

profile_loaded() {
  grep -q '^chariox-app-bwrap ' "$APPARMOR_PROFILES" 2>/dev/null &&
  grep -q '^chariox-app-domain-entry ' "$APPARMOR_PROFILES" 2>/dev/null
}
profile_any_loaded() { grep -Eq '^chariox-app-(bwrap|domain-entry) ' "$APPARMOR_PROFILES" 2>/dev/null; }

install_all() {
  [[ ${#users[@]} -gt 0 ]] || die "name at least one --user"
  [[ -n "$bin" && -n "$runtime" ]] || die "--bin and --runtime are required"
  [[ "$key" =~ ^[0-9a-f]{64}$ ]] || die "--runtime-key must be 64 lowercase hex (the Ed25519 public key)"
  [[ "$digest" =~ ^[0-9a-f]{64}$ ]] || die "--runtime-digest must be 64 lowercase hex (SHA-256 of runtime-inventory.json)"
  [[ -f $R/sys/fs/cgroup/cgroup.controllers ]] || die "cgroup v2 (the unified hierarchy) is required"
  bin=$(realpath -e -- "$bin") || die "no directory $bin"
  runtime=$(realpath -e -- "$runtime") || die "no directory $runtime"
  local name uid controllers controller actual
  for name in chariox-app-storage chariox-app-runtime-install; do
    [[ -f "$bin/$name" && -x "$bin/$name" ]] || die "$bin/$name is missing or not executable"
  done
  for name in runtime-inventory.json runtime-inventory.sig; do
    [[ -f "$runtime/$name" ]] || die "$runtime is not a signed App runtime release (no $name)"
  done
  actual=$(python3 -c 'import hashlib,sys; print(hashlib.sha256(open(sys.argv[1],"rb").read()).hexdigest())' "$runtime/runtime-inventory.json")
  [[ "$actual" == "$digest" ]] || die "runtime-inventory.json has SHA-256 $actual, not the expected $digest"
  for uid in "${uids[@]}"; do
    controllers=" $(systemctl show "user@$uid.service" -p DelegateControllers --value) "
    for controller in cpu memory pids; do
      [[ "$controllers" == *" $controller "* ]] \
        || die "user@$uid.service does not delegate the $controller controller; the kernel's App cgroups need cpu, memory and pids"
    done
  done

  # Both binaries are used from a private root-owned copy from here on, so the
  # source directory cannot swap what is verified, compared or installed.
  local before after status=0 restart=0
  stage=$(mktemp -d)
  trap 'rm -rf -- "$stage"' EXIT
  for name in chariox-app-storage chariox-app-runtime-install; do
    install -m 0700 -- "$bin/$name" "$stage/$name"
  done

  # 1. The signed runtime, verified by the repository's root installer.
  before=$(enrolled_runtime)
  act "verify and install the signed App runtime $digest from $runtime" \
    "$stage/chariox-app-runtime-install" install --source "$runtime" \
    --trusted-public-key-hex "$key" --inventory-sha256 "$digest"
  if (( ! dry_run )); then
    after=$(enrolled_runtime)
    if [[ "$after" == "$before" ]]; then say "unchanged runtime $after (re-verified)"; else say "enrolled runtime $after (was $before)"; fi
  fi

  # 2. Root binaries and the helper's unit.
  put "$stage/chariox-app-runtime-install" "$RUNTIME_INSTALLER" 0755 || true
  put "$stage/chariox-app-storage" "$HELPER" 0755 restart && restart=1
  if put "$here/../managed-kernel/chariox-app-storage.service" "$UNIT" 0644 restart; then
    restart=1
    act "reload systemd units" systemctl daemon-reload
  fi

  # 3. Owners.
  enrollment install "${owner_specs[@]}" || status=$?
  case "$status" in 0) restart=1 ;; 10) ;; *) die "enrollment failed" ;; esac

  # 4. AppArmor.
  if [[ -r $R/sys/module/apparmor/parameters/enabled && "$(cat "$R/sys/module/apparmor/parameters/enabled")" == Y ]]; then
    if put "$here/chariox-app-bwrap.apparmor" "$PROFILE" 0644 || ! profile_loaded; then
      act "load AppArmor profile chariox-app-bwrap" apparmor_parser -r "$PROFILE"
    fi
  else
    say "AppArmor is not enabled; no profile needed"
  fi

  # 5. Lingering.
  local index
  for index in "${!users[@]}"; do
    if [[ -e "$LINGER/${users[$index]}" ]]; then
      say "unchanged lingering for ${users[$index]}"
    else
      act "enable lingering for ${users[$index]}" loginctl enable-linger "${users[$index]}"
      (( dry_run )) || { install -d -m 0755 "$STATE" && : > "$STATE/linger-${uids[$index]}"; }
    fi
  done

  # Recover pre-marker interrupted installs too: compare the running ELF, not
  # the replaced pathname. An unavailable PID/executable also requires activation.
  if systemctl is-active --quiet "$SERVICE"; then
    local main_pid
    main_pid=$(systemctl show "$SERVICE" -p MainPID --value)
    if [[ ! $main_pid =~ ^[1-9][0-9]*$ ]] || ! cmp -s -- "$HELPER" "$R/proc/$main_pid/exe"; then
      restart=1
      restart_obligation
    fi
  fi
  # Retry daemon-reload too: a previous unit replacement may have failed before it.
  if [[ -e "$RESTART_PENDING" ]]; then
    restart=1
    act "reload systemd units for pending helper activation" systemctl daemon-reload
  fi
  # 6. The helper.
  systemctl is-enabled --quiet "$SERVICE" 2>/dev/null || act "enable $SERVICE" systemctl enable --quiet "$SERVICE"
  if ! systemctl is-active --quiet "$SERVICE"; then
    act "start $SERVICE" systemctl start "$SERVICE"
  elif (( restart )); then
    say "restarting $SERVICE: Apps of kernels already running on this machine stop and start again on their next use"
    act "restart $SERVICE" systemctl restart "$SERVICE"
  else
    say "unchanged $SERVICE (running)"
  fi
  (( dry_run )) || systemctl is-active --quiet "$SERVICE" || die "$SERVICE is not running; see journalctl -u $SERVICE"
  (( dry_run )) || rm -f -- "$RESTART_PENDING"
  say "done: runtime $(enrolled_runtime); ${users[*]} can now run a kernel with install-user.sh, without root"
}

uninstall_users() {
  local index uid status=0
  for index in "${!uids[@]}"; do
    uid=${uids[$index]}
    # A running kernel could acquire storage again before the helper restarts.
    [[ ! -d "$R/sys/fs/cgroup/user.slice/user-$uid.slice/user@$uid.service/app.slice/chariox-kernel.service" ]] \
      || die "${users[$index]}'s kernel is running; run install-user.sh uninstall as ${users[$index]} first"
    [[ -z "$(find "$STORAGE/u-$uid" -mindepth 1 -maxdepth 1 2>/dev/null)" ]] \
      || die "${users[$index]} still has App storage in $STORAGE/u-$uid; uninstall their Apps with their data first"
  done
  # The helper refuses to start while a storage directory has no enrolled
  # owner, so each (empty) one goes before its owner does.
  for uid in "${uids[@]}"; do
    if [[ -d "$STORAGE/u-$uid" ]]; then act "remove $STORAGE/u-$uid (empty)" rmdir -- "$STORAGE/u-$uid"; fi
  done
  if [[ -f "$ENROLLMENT" ]]; then
    enrollment remove "${uids[@]}" || status=$?
    case "$status" in 0|10) ;; *) die "enrollment failed" ;; esac
  else
    status=10
  fi
  for index in "${!uids[@]}"; do
    uid=${uids[$index]}
    [[ -e "$STATE/linger-$uid" ]] || continue
    local user; user=$(getent passwd "$uid" | cut -d: -f1 || true)
    [[ -z "$user" ]] || act "disable lingering for $user" loginctl disable-linger "$user"
    remove "$STATE/linger-$uid"
  done
  if (( status == 0 )) && systemctl is-active --quiet "$SERVICE"; then
    if [[ -f "$ENROLLMENT" ]]; then
      say "restarting $SERVICE so it stops serving the removed owners"
      act "restart $SERVICE" systemctl restart "$SERVICE"
    else
      act "stop $SERVICE (no owners are left)" systemctl stop "$SERVICE"
    fi
  fi
  say "done"
}

uninstall_all() {
  local rest lease fd uid user dir
  rest=$(find "$STORAGE" -mindepth 2 -maxdepth 2 2>/dev/null | head -1 || true)
  [[ -z "$rest" ]] || die "App storage still exists ($rest); uninstall every App with its data first"
  # Hold every generation's exclusive runtime lease until this script exits, as
  # the runtime installer's cleanup does: no kernel can take a shared lease on a
  # generation while it is removed. The enrollment goes first, so none opens one.
  for lease in "$RUNTIMES"/*/.runtime-lease; do
    [[ -e "$lease" ]] || continue
    exec {fd}<"$lease"
    flock -n -x "$fd" || die "runtime $(basename "$(dirname "$lease")") is in use; stop every Chariox kernel first"
  done
  if systemctl is-active --quiet "$SERVICE" || systemctl is-enabled --quiet "$SERVICE" 2>/dev/null; then
    act "stop and disable $SERVICE" systemctl disable --now --quiet "$SERVICE"
  fi
  if [[ -e "$UNIT" ]]; then
    remove "$UNIT"
    act "reload systemd units" systemctl daemon-reload
  fi
  if [[ -f "$ENROLLMENT" ]]; then
    for uid in $(python3 -c 'import json,sys; print(" ".join(str(o["uid"]) for o in json.load(open(sys.argv[1]))["owners"]))' "$ENROLLMENT"); do
      [[ -e "$STATE/linger-$uid" ]] || continue
      user=$(getent passwd "$uid" | cut -d: -f1 || true)
      [[ -z "$user" ]] || act "disable lingering for $user" loginctl disable-linger "$user"
      remove "$STATE/linger-$uid"
    done
  fi
  if [[ -e "$PROFILE" ]]; then
    ! profile_any_loaded || act "unload AppArmor profile chariox-app-bwrap" apparmor_parser -R "$PROFILE"
    remove "$PROFILE"
  fi
  remove "$RUNTIME_ENROLLMENT" "$ENROLLMENT" "$HELPER" "$RUNTIME_INSTALLER"
  if [[ -d "$RUNTIMES" ]]; then act "remove $RUNTIMES" rm -rf -- "$RUNTIMES"; fi
  for dir in "$STORAGE"/u-* "$STORAGE" "$STATE" "$R/etc/chariox/apps" "$R/etc/chariox" "$R/usr/lib/chariox"; do
    if [[ -d "$dir" && -z "$(ls -A -- "$dir")" ]]; then act "remove $dir" rmdir -- "$dir"; fi
  done
  say "done: no Chariox system component is left"
}

case "$command" in
  install) install_all ;;
  uninstall)
    if (( all )); then uninstall_all
    elif [[ ${#users[@]} -gt 0 ]]; then uninstall_users
    else die "name --user NAME or --all"
    fi ;;
esac

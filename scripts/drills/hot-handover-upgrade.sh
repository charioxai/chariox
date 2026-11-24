#!/usr/bin/env bash
# Release gate: a planned upgrade (hot reload) keeps every agent running.
#
# Starts the PREVIOUS release's kernel on a COPY of a live kernel's databases, with a running Claude
# turn and a running Codex turn (stand-in provider CLIs: the turn mechanics are real, the model is
# not), hot-upgrades it to the CANDIDATE build and measures, per agent, that the turn kept running:
# no turn lost, no provider process restarted, first output after the swap within the swap time.
#
# Isolation (never the live kernels): the databases are read through SQLite's read-only online
# backup (point --state-db at a backup of the live kernel, not at the live file, when one exists); the drill runs in its own systemd unit and slice with a private PID namespace, a private
# network (loopback only), a read-only host file system and a memory cap.
#
#   hot-handover-upgrade.sh --previous /path/chariox-kernel --candidate /path/chariox-kernel \
#       --test-binary /path/to/hot_reload-<hash> --state-db <kernel.db of a live kernel or its backup> \
#       [--history-db <operational.db>] --home-path <the live kernel's CHARIOX_HOME> \
#       --out /var/tmp/<short>/hhd [--live-kernel-pid PID]
#
# --home-path: the databases bind the absolute paths of their kernel's home (the Skills journal),
#   so the drill mounts the copy at that path, inside its private mount namespace only: the live
#   files there are not visible to the drill.
#
# --out must be short (the drill kernels' Unix socket paths live under it; at most 40 characters).
#
# --test-binary: the `hot_reload` integration test of the candidate tree, built with
#   `cargo test --no-run -p chariox-kernel --test hot_reload` (its path is printed by cargo).
# Exit 0 only if every check in the printed table passes.
set -euo pipefail

previous= candidate= test_binary= state_db= history_db= home_path= out= live_pid=
while [ $# -gt 0 ]; do
  case "$1" in
    --previous) previous=${2:?}; shift 2;;
    --candidate) candidate=${2:?}; shift 2;;
    --test-binary) test_binary=${2:?}; shift 2;;
    --state-db) state_db=${2:?}; shift 2;;
    --history-db) history_db=${2:?}; shift 2;;
    --home-path) home_path=${2:?}; shift 2;;
    --out) out=${2:?}; shift 2;;
    --live-kernel-pid) live_pid=${2:?}; shift 2;;
    *) echo "unknown argument: $1" >&2; exit 2;;
  esac
done
for name in previous candidate test_binary home_path out; do
  [ -n "${!name}" ] || { echo "missing --${name//_/-}" >&2; exit 2; }
done
for file in "$previous" "$candidate" "$test_binary"; do
  [ -x "$file" ] || { echo "not an executable: $file" >&2; exit 2; }
done
[ -n "$state_db$history_db" ] || { echo "missing --state-db or --history-db" >&2; exit 2; }
[ -z "$state_db" ] || [ -f "$state_db" ] || { echo "not a file: $state_db" >&2; exit 2; }
[ -d "$home_path" ] || { echo "not a directory: $home_path" >&2; exit 2; }
[ -z "$history_db" ] || [ -f "$history_db" ] || { echo "not a file: $history_db" >&2; exit 2; }
case "$out" in /*) ;; *) echo "--out must be absolute" >&2; exit 2;; esac
[ ${#out} -le 40 ] || { echo "--out is too long for Unix socket paths: $out" >&2; exit 2; }
[ ! -e "$out" ] || { echo "$out exists: use a new directory" >&2; exit 2; }
# The drill's own directory is the only place it writes or removes anything.
if [ -f /root/.chariox/dev/coordinator/b3tools/safe-paths.sh ]; then
  . /root/.chariox/dev/coordinator/b3tools/safe-paths.sh
  SAFE_ROOTS=$out
  guard_path "$out"
fi
mkdir -p "$out/home" "$out/tmp"
chmod 700 "$out"

version() { "$1" --version 2>&1 | head -1 || true; }
prev_format=$("$previous" --handover-format)
cand_format=$("$candidate" --handover-format)
echo "previous: $(version "$previous") handover-format $prev_format"
echo "candidate: $(version "$candidate") handover-format $cand_format"

# 1. A copy of the live state, through the read-only online backup.
python3 -I - "${state_db:-}" "${history_db:-}" "$out/home" <<'PY'
import hashlib, json, sqlite3, sys, pathlib
from contextlib import closing
sources = {"state/kernel.db": sys.argv[1], "history/operational.db": sys.argv[2]}
dest = pathlib.Path(sys.argv[3]).resolve()
receipt = {}
for relative, origin in sources.items():
    if not origin:
        continue
    origin = pathlib.Path(origin).resolve()
    target = dest / relative
    target.parent.mkdir(parents=True, exist_ok=True)
    with closing(sqlite3.connect(origin.as_uri() + "?mode=ro", uri=True, timeout=15)) as original, \
         closing(sqlite3.connect(target)) as copy:
        original.backup(copy)
    with closing(sqlite3.connect(target.as_uri() + "?mode=ro", uri=True)) as saved:
        assert saved.execute("PRAGMA quick_check").fetchall() == [("ok",)], relative
    receipt[relative] = {"source": str(origin), "bytes": target.stat().st_size,
                         "sha256": hashlib.sha256(target.read_bytes()).hexdigest()}
assert receipt, "no database copied"
(dest.parent / "state-copy.json").write_text(json.dumps(receipt, indent=1))
print("state copy:", json.dumps(receipt))
PY

provider_pids() { [ -n "$live_pid" ] && ps -o pid=,lstart= --ppid "$live_pid" | sort || true; }
provider_pids > "$out/live-children-before.txt"

# 2. The drill, isolated: own unit and slice, private PID and network namespaces, read-only host.
unit=hot-handover-drill-$$-$(date +%s)
set +e
systemd-run --unit="$unit" --slice=hot-handover-drill.slice --collect --wait --pipe \
  -p PrivatePIDs=yes -p PrivateNetwork=yes -p ProtectSystem=strict -p ProtectHome=read-only \
  -p NoNewPrivileges=yes -p TemporaryFileSystem=/tmp -p ReadWritePaths="$out $home_path" -p BindPaths="$out/home:$home_path" \
  -p MemoryMax=12G -p TasksMax=4000 \
  env -i PATH=/usr/bin:/bin HOME="$out/tmp" TMPDIR="$out/tmp" \
    CHARIOX_HOT_RELOAD_KERNEL="$previous" \
    CHARIOX_HOT_RELOAD_UPGRADE_TO="$candidate" \
    CHARIOX_HOT_RELOAD_HOME="$home_path" CHARIOX_HOT_RELOAD_KEEP=1 \
    CHARIOX_HOT_RELOAD_CAPTURE="$out/handover-captured.json" \
    CHARIOX_HOT_RELOAD_MEASURE="$out/measure.json" \
    "$test_binary" a_codex_turn_continues_beside_a_running_claude_turn --exact --test-threads=1 --nocapture \
  > "$out/drill.log" 2>&1
rc=$?
set -e
provider_pids > "$out/live-children-after.txt"
echo "drill unit $unit rc=$rc (log $out/drill.log)"

# 3. The numbers, per agent.
python3 -I - "$out" "$rc" "$prev_format" "$cand_format" <<'PY'
import json, pathlib, sys
out, rc = pathlib.Path(sys.argv[1]), int(sys.argv[2])
checks = []
def check(name, ok, detail):
    checks.append({"check": name, "ok": bool(ok), "detail": detail})
check("drill test passed", rc == 0, f"exit {rc}")
try:
    m = json.loads((out / "measure.json").read_text())
except Exception as error:
    m = {}
    check("measurements written", False, str(error))
codex, claude = m.get("codex", {}), m.get("claude") or {}
check("Codex turn completed after the swap, every part once", codex.get("turn_completed") is True,
      f"parts in history {codex.get('parts_in_history')}")
check("no Codex app-server restarted", codex.get("app_server_starts_after_swap") == 0,
      f"starts after the swap {codex.get('app_server_starts_after_swap')}")
check("Codex first output after the swap within the swap time",
      codex.get("longest_stall_ms") is not None and codex.get("swap_ms") is not None
      and codex["longest_stall_ms"] <= codex["swap_ms"] + 1000,
      f"swap {codex.get('swap_ms')} ms, longest stall {codex.get('longest_stall_ms')} ms")
check("Claude turn kept its process (one start)", claude.get("process_starts") == 1,
      f"starts {claude.get('process_starts')}")
check("Claude turn got its prompt once", claude.get("prompts") == 1, f"prompts {claude.get('prompts')}")
check("Claude turn lost no part", claude.get("parts") == 400, f"parts {claude.get('parts')}")
check("Claude turn finished promptly after release",
      claude.get("turn_completed_after_release_ms") is not None
      and claude["turn_completed_after_release_ms"] < 30000,
      f"{claude.get('turn_completed_after_release_ms')} ms")
before = (out / "live-children-before.txt").read_text()
after = (out / "live-children-after.txt").read_text()
check("live kernel's children unchanged", before == after, f"{len(before.splitlines())} children")
captured = out / "handover-captured.json"
check("the previous release's handover document was captured", captured.is_file(), str(captured))
result = {"prev_format": sys.argv[3], "cand_format": sys.argv[4], "measure": m, "checks": checks}
(out / "result.json").write_text(json.dumps(result, indent=1))
width = max(len(c["check"]) for c in checks)
for c in checks:
    print(("PASS" if c["ok"] else "FAIL"), c["check"].ljust(width), c["detail"])
sys.exit(0 if all(c["ok"] for c in checks) else 1)
PY

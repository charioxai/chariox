#!/usr/bin/env bash
# Release gate: agents recover quickly after a kernel cold restart.
#
# Boots a kernel binary on a COPY of a live kernel's state, cold-restarts it, and
# times every agent's way back to work after each boot: the restart recovery, the
# rebuilt handoff (history read, brief, render) and the first provider output.
# The live kernel is only read (SQLite online backup of its two databases).
#
# The copy runs in its own systemd unit and slice, in private PID and network
# namespaces, with /root read-only: it cannot reach Cloud, a relay, a provider
# or any live process, cgroup or file. Inside its unit the copy is mounted over
# the source home's path, because the state pins that path; the live files stay
# untouched and unseen behind that mount. Claude runs are answered by a stand-in
# CLI that replies at once, so the timings are the kernel's own work.
#
# Run as root, outside any kernel's cgroup:
#   systemd-run --unit=restart-stall-drill --collect --property=Type=exec \
#     scripts/drills/restart-stall/run.sh --binary <kernel> --source-home <CHARIOX_HOME> \
#       --kernel-id <id the source kernel reports> --work /var/tmp/<new dir> [--window 600] \
#       [--sessions lost|kept] [--probe-ipc <kernel-client dist/ipc.js> --probe-room <room id>]
# With --probe-*: the two Claude agents with the longest histories and three
# small ones get a probe prompt (--probe-owner <the Rooms' owner user id>: the
# copy is not enrolled in Cloud, so its local client acts as that owner only
# after the copy's databases name the local owner instead; done on the copy) once the first boot settled and again right as
# the second boot is ready; report.md lists each probe's time to reach its
# provider (the stand-in answers at once, so that is its time to first output).
# --second-binary B: the second boot runs B on what the first left, i.e. a
# cold switch between kernels (an upgrade, or a rollback) on the same state.
# --latency-runs N (with --probe-*): LAT-1 instead of the restarts. One boot;
# N times, with the longest-history agent streaming a 15 s turn (busy) and
# with it idle: spawn a fresh Claude Haiku agent and prompt "hi there"; the
# report gives prompt -> reply recorded by the kernel (what clients show).
# --sessions lost (default): no Claude session can be resumed, so every agent
# with work rebuilds its context from history, the worst case of a restart.
# Results: <work>/report.md (per boot: per-agent table, kernel CPU), raw logs beside it.
set -euo pipefail

window=600
sessions=lost
while [[ $# -gt 0 ]]; do
  case "$1" in
    --binary) binary=$(realpath -e "$2"); shift 2;;
    --second-binary) second_binary=$(realpath -e "$2"); shift 2;;
    --source-home) source_home=$(realpath -e "$2"); shift 2;;
    --kernel-id) kernel_id=$2; shift 2;;
    --work) work=$2; shift 2;;
    --window) window=$2; shift 2;;
    --sessions) sessions=$2; shift 2;;
    --probe-ipc) probe_ipc=$(realpath -e "$2"); shift 2;;
    --probe-room) probe_room=$2; shift 2;;
    --probe-owner) probe_owner=$2; shift 2;;
    --latency-runs) latency_runs=$2; shift 2;;
    --seed-values) seed_values=$2; shift 2;;
    *) echo "unknown argument $1" >&2; exit 2;;
  esac
done
: "${binary:?--binary}" "${source_home:?--source-home}" "${kernel_id:?--kernel-id}" "${work:?--work}"
[[ "$(id -u)" == 0 ]] || { echo "run as root (systemd units, namespaces)" >&2; exit 2; }
[[ "$work" == /var/tmp/* && ! -e "$work" ]] || { echo "--work must be a new directory under /var/tmp" >&2; exit 2; }
[[ "$sessions" == lost || "$sessions" == kept ]] || { echo "--sessions lost|kept" >&2; exit 2; }
[[ "$kernel_id" =~ ^kernel-[0-9a-f]+$ ]] || { echo "--kernel-id looks like kernel-<hex>" >&2; exit 2; }
awk '{ exit !($2 < 32) }' /proc/loadavg || { echo "load5 >= 32: not now" >&2; exit 3; }
awk '/MemAvailable/ { exit !($2 > 15 * 1024 * 1024) }' /proc/meminfo || { echo "MemAvailable <= 15 GB: not now" >&2; exit 3; }

here=$(dirname "$(realpath -e "$0")")
# Every path this script writes, moves or removes lies inside its own new work
# directory; anything else is refused before it is touched.
work=$(realpath -m "${work:?}")
[[ "$work" == /var/tmp/?*/?* ]] || { echo "--work must be at least two levels under /var/tmp" >&2; exit 2; }
inside_work() {
  local path=${1:?empty path}
  [[ "$path" == "$work"/?* && "/$path/" != */../* ]] || { echo "refused: '$path' is outside $work" >&2; exit 97; }
}
remove_contents() {  # $1 = a directory inside the work directory
  inside_work "$1"
  find "$1" -mindepth 1 -maxdepth 1 -exec rm -rf --one-file-system -- {} +
}
umask 077
home=$work/home
inside_work "$home"
mkdir -p "$home/state" "$home/history" "$home/kernels" "$work/bin" "$work/logs" "$work/os-home" "$work/tmp"
port=56201
slice=rsdrill$$.slice  # one per run, so two drills never clean up each other's cgroups; no dash: a dash nests a slice
log() { printf '%s %s\n' "$(date -u +%FT%TZ)" "$*" | tee -a "$work/drill.log"; }

# The stand-in Claude CLI: every user message gets an answer and a result. A
# brief request gets a well-formed brief, so the kernel stores it.
inside_work "$work/bin/claude"
{ head -1 "$here/claude-standin.sh"; echo "sessions=$sessions"; echo "probes=$work/probes-arrived.tsv"
  tail -n +2 "$here/claude-standin.sh"; } > "$work/bin/claude"
chmod 755 "$work/bin/claude"
cp -- "$here/mcp_vault_seed.py" "$work/bin/mcp_vault_seed.py"

start_kernel() {  # $1 = unit name
  systemd-run --unit="$1" --slice="$slice" --collect --property=Type=exec \
    --property=PrivateNetwork=yes --property=TemporaryFileSystem=/tmp --property=ProtectHome=read-only \
    --property=ReadWritePaths="$work" --property=BindPaths="$home:$source_home" \
    --property=KillMode=control-group \
    unshare --pid --fork --mount-proc --kill-child \
    env -i PATH=/usr/local/bin:/usr/bin:/bin HOME="$work/os-home" TMPDIR="$work/tmp" \
      CHARIOX_HOME="$source_home" CHARIOX_LOG_DIR="$work/logs" CHARIOX_KERNEL_PORT=$port \
      CHARIOX_CLAUDE_BIN="$work/bin/claude" RUST_BACKTRACE=1 \
      "${kernel_binary:-$binary}" > /dev/null
}

stop_kernel() {  # $1 = unit name; a cold stop: every process of the unit at once
  systemctl kill --signal=SIGKILL "$1" 2>/dev/null || true
  for _ in $(seq 1 60); do systemctl is-active -q "$1" || return 0; sleep 1; done
  echo "unit $1 did not stop" >&2; return 1
}

watch_cpu() {  # $1 = unit, $2 = seconds, $3 = output tsv
  local cg; cg=/sys/fs/cgroup/$slice/$1.service
  printf 'elapsed_s\tcpu_s\n' > "$3"
  for t in $(seq 0 5 "$2"); do
    [[ -r $cg/cpu.stat ]] && printf '%s\t%s\n' "$t" "$(awk '/usage_usec/ { printf "%.1f", $2 / 1e6 }' "$cg/cpu.stat")" >> "$3"
    sleep 5
  done
}

# 1. A fresh identity for the copy (its own keys), then the source kernel's public
#    ID, under which its Rooms were stored.
unit=restart-stall-identity-$$
start_kernel "$unit"
for _ in $(seq 1 60); do [[ -s $home/kernels/registry.json ]] && break; sleep 1; done
stop_kernel "$unit"
python3 - "$home/kernels/registry.json" "$kernel_id" <<'PY'
import json, sys
path, kernel_id = sys.argv[1:]
registry = json.load(open(path))
assert len(registry["kernels"]) == 1, "one kernel identity expected"
for record in registry["kernels"].values():
    record["kernel_id"] = kernel_id
json.dump(registry, open(path, "w"))
PY
remove_contents "$work/logs"; remove_contents "$home/state"; remove_contents "$home/history"
log "copy identity ready: $kernel_id with its own keys"

# 2. The source state, read through SQLite online backups.
python3 - "$source_home" "$home" <<'PY'
import sqlite3, sys
src, dst = sys.argv[1:]
for rel in ("state/kernel.db", "history/operational.db"):
    s = sqlite3.connect(f"file:{src}/{rel}?mode=ro", uri=True, timeout=60)
    d = sqlite3.connect(f"{dst}/{rel}")
    with d:
        s.backup(d)
    assert d.execute("PRAGMA quick_check").fetchone()[0] == "ok", rel
    s.close(); d.close()
PY
# The configuration and the account registry (which profile each agent runs on)
# come along; no Vault file, relay identity or provider credential does. The
# stand-in CLI needs no login.
if [[ -f $source_home/config.toml ]]; then inside_work "$home/config.toml"; cp -- "$source_home/config.toml" "$home/config.toml"; fi
# Installed skill packages and agent guides: a new run reloads them.
for kept in state/skills skills metaagent-guides workflow-instructions; do
  if [[ -d $source_home/$kept ]]; then
    inside_work "$home/$kept"; mkdir -p "$(dirname "$home/$kept")"; cp -a -- "$source_home/$kept" "$(dirname "$home/$kept")/"
  fi
done
if [[ -f $source_home/state/provider-accounts.json ]]; then
  # Each profile's login directory becomes an empty directory inside the copy.
  python3 - "$source_home" "$home" <<'PY'
import json, os, sys
source, copy = sys.argv[1:]
def inside_copy(path):
    resolved = os.path.realpath(path)
    if not resolved.startswith(copy + "/"):
        sys.exit(f"refused: {path!r} is outside {copy}")
    return resolved
document = json.load(open(f"{source}/state/provider-accounts.json"))
for profile in document.get("profiles", []):
    locator = profile.get("locator", {})
    for key, value in list(locator.items()):
        if isinstance(value, str) and value.startswith("/"):
            inside = value if value.startswith(source + "/") else f"{source}/drill-outside{value}"
            locator[key] = inside
            os.makedirs(inside_copy(copy + inside[len(source):]), mode=0o700, exist_ok=True)
    # The stand-in CLI needs no login, but a launch requires a login file: a
    # fixed placeholder, never a credential.
    if profile.get("provider") == "claude" and "claude_config_dir" in locator:
        placeholder = inside_copy(copy + locator["claude_config_dir"][len(source):] + "/.credentials.json")
        with open(os.open(placeholder, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600), "w") as handle:
            json.dump({"claudeAiOauth": {"accessToken": "drill-placeholder",
                "refreshToken": "drill-placeholder", "expiresAt": 4102444800000}}, handle)
# The copy is not enrolled in Cloud, so its Rooms' owner is not its local owner:
# file the local profiles under every other owner the registry knows as well.
profiles = document.get("profiles", [])
owners = {profile["owner_user_id"] for profile in profiles} - {"local"}
known = {(p["owner_user_id"], p["provider"], p["profile_id"]) for p in profiles}
for owner in owners:
    for profile in [p for p in profiles if p["owner_user_id"] == "local"]:
        if (owner, profile["provider"], profile["profile_id"]) not in known:
            profiles.append(dict(profile, owner_user_id=owner, is_default=False))
json.dump(document, open(f"{copy}/state/provider-accounts.json", "w"))
PY
fi
log "state copied: $(du -sh "$home" | cut -f1)"
if [[ -n "${probe_owner:-}" ]]; then
  [[ "$probe_owner" =~ ^[a-z0-9]+$ ]] || { echo "--probe-owner looks like a user id" >&2; exit 2; }
  python3 - "$home" "$probe_owner" <<'PY'
import sqlite3, sys
home, owner = sys.argv[1:]
for rel in ("state/kernel.db", "history/operational.db"):
    db = sqlite3.connect(f"{home}/{rel}")
    for (table,) in db.execute("SELECT name FROM sqlite_master WHERE type = 'table'").fetchall():
        for column in [row[1] for row in db.execute(f"PRAGMA table_info('{table}')")]:
            db.execute(f"UPDATE '{table}' SET \"{column}\" = REPLACE(\"{column}\", ?, 'local')"
                       f" WHERE typeof(\"{column}\") = 'text' AND instr(\"{column}\", ?) > 0", (owner, owner))
    db.commit(); db.close()
PY
  log "copy owned by its local owner"
fi
probe_agents=()
if [[ -n "${probe_ipc:-}" ]]; then
  : "${probe_room:?--probe-room}"
  mapfile -t probe_agents < <(python3 - "$home/history/operational.db" "$probe_room" "$work/probe-agents.tsv" <<'PY'
import json, sqlite3, sys
db, room, out = sys.argv[1:]
history = sqlite3.connect(f"file:{db}?mode=ro", uri=True)
rows = history.execute(
    "SELECT agent_id, COUNT(*), SUM(LENGTH(event_json)) FROM history_events"
    " WHERE session_id = ? AND agent_id IS NOT NULL GROUP BY agent_id", (room,)).fetchall()
def claude(agent):
    row = history.execute(
        "SELECT event_json FROM history_events WHERE session_id = ? AND agent_id = ?"
        " AND kind = 'provider_output' ORDER BY sequence DESC LIMIT 1", (room, agent)).fetchone()
    event = json.loads(row[0]) if row else {}
    return "claude" in (event.get("provider"), event.get("metadata", {}).get("external_provider"))
rows = [row for row in rows if row[1] >= 20 and claude(row[0])]
rows.sort(key=lambda row: row[2])
chosen = rows[-2:] + rows[:3]
with open(out, "w") as handle:
    for agent, events, size in chosen:
        handle.write(f"{agent}\t{events}\t{size}\n")
        print(agent)
PY
)
  log "probe agents: ${#probe_agents[@]} ($work/probe-agents.tsv)"
fi
probe() {  # $1 = unit, $2 = phase
  [[ ${#probe_agents[@]} -gt 0 ]] || return 0
  local main kernel
  main=$(systemctl show -p MainPID --value "$1")
  kernel=$(pgrep -P "$main" | head -1)
  [[ -n "$kernel" ]] || { log "probe $2: no kernel process"; return 0; }
  nsenter -t "$kernel" --net --mount node "$here/probe.mjs" "$probe_ipc" "$source_home" "$port" \
    "$probe_room" "$2" "$work/probes-sent.tsv" "${probe_agents[@]}" || log "probe $2 failed"
  log "probe $2 sent"
}
wait_ready() {  # $1 = seconds; the kernel accepts local commands
  local t
  for t in $(seq 1 "$1"); do
    grep -qs '"kernel ready for local command"' "$work/logs"/*daemon*.ndjson && return 0
    sleep 1
  done
  return 1
}

if [[ -n "${latency_runs:-}" ]]; then
  [[ ${#probe_agents[@]} -gt 0 && "$latency_runs" =~ ^[0-9]+$ ]] || { echo "--latency-runs needs --probe-* and a count" >&2; exit 2; }
  big=$(sort -t$'\t' -k3,3n "$work/probe-agents.tsv" | tail -1 | cut -f1)
  unit=restart-stall-latency-$$
  start_kernel "$unit"
  watch_cpu "$unit" 3600 "$work/cpu-latency.tsv" &
  watcher=$!
  wait_ready 300 || { log "kernel not ready"; stop_kernel "$unit"; exit 1; }
  sleep 90
  main=$(systemctl show -p MainPID --value "$unit"); kernel=$(pgrep -P "$main" | head -1)
  # A working Room holds protected values (live 20A7: about 20 Vault entries);
  # the copy gets --seed-values (default 25) fresh generated ones through the
  # agents' Vault tool. A Room holds at most 256.
  seeder=$(sort -t$'\t' -k3,3n "$work/probe-agents.tsv" | head -1 | cut -f1)
  nsenter -t "$kernel" --net --mount node "$here/lat1.mjs" "$probe_ipc" "$source_home" "$port" \
    "$probe_room" "$seeder" seed "${seed_values:-25}" "$work/latency-seed.tsv" || log "seed failed"
  for _ in $(seq 1 120); do
    [[ $(grep -c '"result"' "$work/seed.log" 2>/dev/null) -ge ${seed_values:-25} ]] && break
    sleep 5
  done
  log "seeded: $(grep -c '"result"' "$work/seed.log" 2>/dev/null) vault tool results"
  for run in $(seq 1 "$latency_runs"); do
    for mode in busy idle; do
      nsenter -t "$kernel" --net --mount node "$here/lat1.mjs" "$probe_ipc" "$source_home" "$port" \
        "$probe_room" "$big" "$mode" "$run" "$work/latency-sent.tsv" || log "lat1 $mode $run failed"
      log "lat1 $mode $run sent"
      sleep 60
    done
  done
  kill "$watcher" 2>/dev/null || true
  stop_kernel "$unit"
  python3 - "$work" "$home/history/operational.db" "$big" > "$work/report.md" <<'PY'
import sqlite3, sys
work, db, big = sys.argv[1:]
history = sqlite3.connect(f"file:{db}?mode=ro", uri=True)
size = history.execute("SELECT COUNT(*), SUM(LENGTH(event_json)) FROM history_events WHERE agent_id = ?", (big,)).fetchone()
print(f"## LAT-1: prompt 'hi there' to a fresh Claude Haiku agent -> reply recorded by the kernel\n")
print(f"Busy agent: {big} ({size[0]} events, {size[1] / 1e6:.1f} MB), streaming a 15 s turn that ends ~3 s after the spawn.\n")
print("| run | big agent | spawn (s) | prompt -> reply shown (s) |")
print("|---|---|---|---|")
for line in open(f"{work}/latency-sent.tsv").read().splitlines():
    run, mode, agent, spawn_sent, spawn_done, prompt_sent, submitted = line.split("\t")
    if agent == "failed":
        print(f"| {run} | {mode} | failed | {submitted} |"); continue
    row = history.execute("SELECT MIN(timestamp_ms) FROM history_events WHERE agent_id = ? AND kind = 'provider_output'", (agent,)).fetchone()
    reply = (row[0] - int(prompt_sent)) / 1000 if row and row[0] else "never"
    print(f"| {run} | {mode} | {(int(spawn_done) - int(spawn_sent)) / 1000} | {reply} |")
PY
  log "done: $work/report.md"
  exit 0
fi

# 3. Two cold boots of the copy: the first is the restart the source state was
#    captured in, the second restarts what the first left.
for boot in 1 2; do
  kernel_binary=$binary
  if [[ $boot == 2 && -n "${second_binary:-}" ]]; then kernel_binary=$second_binary; fi
  unit=restart-stall-boot$boot-$$
  boot_ms=$(date +%s%3N)
  log "boot $boot: starting $unit ($(basename "$kernel_binary"))"
  start_kernel "$unit"
  watch_cpu "$unit" "$window" "$work/cpu-boot$boot.tsv" &
  watcher=$!
  if wait_ready 300; then
    # The first boot is the baseline: probe once its recovery settled. The
    # second boot is the restart under test: probe the moment it is ready.
    if [[ $boot == 1 ]]; then sleep 90; fi
    probe "$unit" "boot$boot"
  fi
  wait "$watcher"
  stop_kernel "$unit"
  log "boot $boot: cold-stopped after ${window}s"
  inside_work "$work/boot$boot"; mkdir -p "$work/boot$boot"
  find "$work/logs" -mindepth 1 -maxdepth 1 -exec mv -- {} "$work/boot$boot/" \;
  python3 "$here/report.py" "$work/boot$boot" "$boot_ms" "$work/cpu-boot$boot.tsv" >> "$work/report.md"
done
if [[ -s $work/probes-sent.tsv ]]; then
  python3 - "$work" >> "$work/report.md" <<'PY'
import sys
work = sys.argv[1]
sizes = {line.split("\t")[0]: line.split("\t")[1:] for line in open(f"{work}/probe-agents.tsv").read().splitlines()}
arrived = {}
for line in open(f"{work}/probes-arrived.tsv").read().splitlines() if __import__("os").path.exists(f"{work}/probes-arrived.tsv") else []:
    marker, at = line.split("\t")
    arrived.setdefault(marker, int(at))
print("## Probes: time from prompt to its provider (first output)\n")
print("| agent | events | history MB | boot | sent as | first output after (s) |")
print("|---|---|---|---|---|---|")
for line in open(f"{work}/probes-sent.tsv").read().splitlines():
    marker, sent, result = line.split("\t")
    _, _, phase, agent = marker.split("-", 3)
    events, size = sizes.get(agent, ["?", "0"])
    took = (arrived[marker] - int(sent)) / 1000 if marker in arrived else "never"
    print(f"| {agent[:14]} | {events} | {int(size) / 1e6:.1f} | {phase} | {result} | {took} |")
print()
PY
fi
log "done: $work/report.md"

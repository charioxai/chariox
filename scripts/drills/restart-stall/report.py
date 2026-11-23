#!/usr/bin/env python3
"""Per-agent timings of one boot of the restart drill, from the kernel's own log."""
import glob, json, sys

logs, boot_ms, cpu_tsv = sys.argv[1], int(sys.argv[2]), sys.argv[3]
events = []
for path in sorted(glob.glob(f"{logs}/*daemon*.ndjson")):
    with open(path, errors="replace") as handle:
        for line in handle:
            try:
                events.append(json.loads(line))
            except ValueError:
                pass
events.sort(key=lambda event: event.get("timestamp_ms", 0))

def seconds(event):
    return (event["timestamp_ms"] - boot_ms) / 1000

agents, recovery = {}, {}
counts = {"provider launches": 0, "handoffs rebuilt": 0, "briefs stored": 0, "briefs failed": 0}
for event in events:
    message = event.get("message", "")
    counts["provider launches"] += message == "provider launch planned"
    counts["handoffs rebuilt"] += message == "transferring the agent conversation to a new provider session"
    counts["briefs stored"] += message == "updated the agent's handoff brief"
    counts["briefs failed"] += message.startswith("failed to update the agent's handoff brief")
    if message == "captured durable restart recovery targets":
        recovery["targets"] = event.get("active_prompt_targets")
        recovery["captured_s"] = seconds(event)
    elif message == "reconciled durable runtime work after kernel restart":
        recovery["reconciled_s"] = seconds(event)
        recovery["continuations"] = event.get("provider_continuations_dispatched")
        recovery["interrupted"] = event.get("unrecoverable_local_prompts_cancelled")
    agent = event.get("agent_id")
    if not agent:
        continue
    row = agents.setdefault(agent, {})
    if message == "transferring the agent conversation to a new provider session":
        row.setdefault("derive_ms", event.get("derive_ms"))
        row.setdefault("handoff_s", seconds(event))
    elif message == "updated the agent's handoff brief":
        row.setdefault("brief_s", seconds(event))
        row.setdefault("brief", f"stored ({event.get('utility_calls')} calls)")
    elif message.startswith("failed to update the agent's handoff brief"):
        row.setdefault("brief_s", seconds(event))
        row.setdefault("brief", "failed: " + event.get("error", "")[-60:])
    elif message == "turn waited before its first visible output":
        row.setdefault("first_output_s", seconds(event))

cpu = [line.split("\t") for line in open(cpu_tsv).read().split("\n")[1:] if line]
print(f"## {logs}\n")
print(f"Recovery: {json.dumps(recovery)}\n")
print(f"Counts: {json.dumps(counts)}\n")
ready = [row["first_output_s"] for row in agents.values() if "first_output_s" in row]
derives = [row["derive_ms"] for row in agents.values() if row.get("derive_ms") is not None]
print(f"Agents with first output: {len(ready)}, last at {max(ready, default='-')} s; "
      f"slowest handoff derivation: {max(derives, default='-')} ms\n")
print("| agent | handoff ready (s) | derive (ms) | brief | brief (s) | first output (s) |")
print("|---|---|---|---|---|---|")
for agent, row in sorted(agents.items(), key=lambda item: item[1].get("first_output_s", 1e9)):
    if not row:
        continue
    print(f"| {agent[:14]} | {row.get('handoff_s', '-')} | {row.get('derive_ms', '-')} | "
          f"{row.get('brief', '-')} | {row.get('brief_s', '-')} | {row.get('first_output_s', '-')} |")
if cpu:
    print("\nKernel CPU seconds by elapsed seconds: " + ", ".join(f"{t}s:{c}" for t, c in cpu[::6]))
    if len(cpu) > 13:
        (t0, c0), (t1, c1) = cpu[-13], cpu[-1]
        print(f"Kernel CPU over the last minute: {(float(c1) - float(c0)) / (int(t1) - int(t0)):.2f} cores")
print()

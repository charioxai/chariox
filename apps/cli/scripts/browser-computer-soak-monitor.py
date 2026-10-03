#!/usr/bin/env python3
"""MP-07 / MP-08 / MP-10: exact-owned detached Docker soak monitor."""
import datetime
import json
import os
import pathlib
import shutil
import subprocess
import sys
import time

MP = ["MP-07", "MP-08", "MP-10"]


def checkpoint_state(status, browser, now):
    """An explicit fresh restart checkpoint may outlive its old browser PID."""
    checkpoint = status["checkpoint"]
    age = now - datetime.datetime.fromisoformat(status["updatedAt"].replace("Z", "+00:00")).timestamp()
    if age < -5 or age > 120:
        raise RuntimeError("runner checkpoint is stale")
    if browser and browser["state"] != "Z":
        if str(browser["startTime"]) != str(checkpoint["browserStartTime"]):
            raise RuntimeError("browser PID identity changed")
        return "healthy"
    if checkpoint["label"] == "before_restart":
        return "restarting"
    raise RuntimeError("browser is not live outside controlled restart")


def main():
    config = json.loads(pathlib.Path(sys.argv[1]).read_text())
    evidence = pathlib.Path(config["evidence"])
    cid = config["containerId"]
    assert len(cid) == 64 and all(c in "0123456789abcdef" for c in cid)
    started = time.monotonic()
    errors = 0
    guard_at = None
    runner_identity = None
    controller_identity = None
    previous_checkpoint = None
    current_run = None

    def now():
        return datetime.datetime.now(datetime.timezone.utc).isoformat()

    def append(name, value):
        with (evidence / name).open("a") as handle:
            handle.write(json.dumps({"mp": MP, "at": now(), **value}) + "\n")

    def command(args):
        result = subprocess.run(args, capture_output=True, text=True, timeout=30)
        if args[1] != "inspect":
            append("monitor-actions.jsonl", {"command": args, "exitCode": result.returncode})
        return result

    def inspect():
        result = command(["docker", "inspect", "--format", "{{json .}}", cid])
        result.check_returncode()
        value = json.loads(result.stdout)
        labels = value["Config"]["Labels"]
        if (value["Id"] != cid or value["Image"] != config["imageId"]
                or labels.get("io.chariox.drill-owner") != config["owner"]
                or labels.get("io.chariox.soak-protect") != config["protection"]):
            raise RuntimeError("monitor refuses changed container ownership")
        return value

    def identity(container, pid):
        try:
            fields = pathlib.Path(f"/proc/{container['State']['Pid']}/root/proc/{pid}/stat").read_text().rsplit(")", 1)[1].split()
            return {"pid": pid, "state": fields[0], "startTime": fields[19]}
        except (OSError, IndexError):
            return None

    def live(container, pid, expected=None):
        observed = identity(container, pid)
        if not observed or observed["state"] == "Z" or (expected and observed["startTime"] != expected["startTime"]):
            raise RuntimeError("owned process identity is not live")
        return observed

    (evidence / "host-monitor.pid").write_text(str(os.getpid()) + "\n")
    while True:
        container = inspect()
        run_dirs = sorted((p for p in evidence.iterdir() if p.is_dir()), reverse=True)
        latest = run_dirs[0] if run_dirs else None
        if latest != current_run:
            # Preflight, smoke and detach intentionally have different process trees.
            current_run = latest
            runner_identity = None
            controller_identity = None
            previous_checkpoint = None
            errors = 0
            append("monitor-actions.jsonl", {"action": "run_directory_changed", "runDir": str(latest)})
        status = None
        if latest and (latest / "status.json").exists():
            status = json.loads((latest / "status.json").read_text())
        mem = int(next(line.split()[1] for line in pathlib.Path("/proc/meminfo").read_text().splitlines() if line.startswith("MemAvailable:"))) * 1024
        sample = {"diskFreeBytes": shutil.disk_usage("/").free, "memAvailableBytes": mem,
                  "containerId": cid, "running": container["State"]["Running"], "status": status.get("status") if status else None}
        append("host-resources.jsonl", sample)
        if not container["State"]["Running"]:
            result = json.loads((latest / "result.json").read_text()) if latest and (latest / "result.json").exists() else None
            logs = command(["docker", "logs", cid])
            (evidence / "container.log").write_text(logs.stdout + logs.stderr)
            command(["docker", "rm", cid]).check_returncode()
            final = {"mp": MP, "at": now(), "scope": config["scope"], "gateEligible": False,
                     "exitCode": container["State"]["ExitCode"], "oomKilled": container["State"]["OOMKilled"],
                     "resultStatus": result.get("status") if result else None,
                     "failure": result.get("failure") if result else "missing terminal result",
                     "cleanupClean": result.get("cleanup", {}).get("clean") is True if result else False,
                     "redactionPassed": result.get("redaction", {}).get("passed") if result else False,
                     "containerRemoved": True, "monitorStopTriggered": guard_at is not None,
                     "runDir": str(latest)}
            (evidence / "host-completion.json").write_text(json.dumps(final, indent=2) + "\n")
            return
        reason = None
        if sample["diskFreeBytes"] < 11 * 1024**3 or mem < 5 * 1024**3:
            reason = "host reserve approached"
        try:
            if status and status["status"] == "healthy":
                runner_identity = live(container, status["pid"], runner_identity)
                if config.get("mode", "idle") == "active":
                    age = time.time() - datetime.datetime.fromisoformat(status["updatedAt"].replace("Z", "+00:00")).timestamp()
                    if age < -5 or age > 120:
                        raise RuntimeError("active runner status is stale")
                    controller_identity = live(container, status["firstHealth"]["process_id"], controller_identity)
                    cp = status["activity"]
                    elapsed = round((time.monotonic() - started) * 1000)
                    counters = status["activity"]
                    observed = "healthy"
                else:
                    cp = status["checkpoint"]
                    controller_identity = live(container, cp["controllerPid"], controller_identity)
                    if str(controller_identity["startTime"]) != str(cp["controllerStartTime"]):
                        raise RuntimeError("controller PID identity changed")
                    observed = checkpoint_state(status, identity(container, cp["browserPid"]), time.time())
                    elapsed = status["elapsedMonotonicMs"]
                    counters = status["counters"]
                if cp != previous_checkpoint:
                    append("operator-checkpoints.jsonl", {"state": observed, "scope": config["scope"], "gateEligible": False,
                           "elapsedMonotonicMs": elapsed, "checkpoint": cp, "counters": counters})
                    previous_checkpoint = cp
                if config.get("validateStop") and (latest / "detach-contract.json").exists():
                    reason = "synthetic owned stop-path validation"
            elif time.monotonic() - started > 180 and (not status or status["status"] == "starting"):
                raise RuntimeError("runner failed to reach healthy checkpoint")
            errors = 0
        except (OSError, ValueError, KeyError, RuntimeError) as error:
            errors += 1
            append("monitor-actions.jsonl", {"action": "observation_error", "message": str(error), "consecutiveErrors": errors})
            if errors >= 3:
                reason = "three consecutive observation failures"
        if reason and guard_at is None:
            guard_at = time.monotonic()
            append("monitor-actions.jsonl", {"action": "stop_requested", "reason": reason, "sample": sample})
            if status and runner_identity:
                try:
                    live(container, status["pid"], runner_identity)
                except RuntimeError:
                    # A dead/reused PID is never signalled; retain the bounded
                    # exact-container fallback instead of crashing the monitor.
                    append("monitor-actions.jsonl", {"action": "runner_not_signalable"})
                else:
                    command(["docker", "exec", cid, "/bin/kill", "-TERM", str(status["pid"])])
        if guard_at is not None and time.monotonic() - guard_at > 90:
            inspect()
            command(["docker", "stop", "--time", "10", cid]).check_returncode()
        (evidence / "monitor-status.json").write_text(json.dumps({"mp": MP, "at": now(), "pid": os.getpid(), "consecutiveErrors": errors,
                                                                "stopTriggered": guard_at is not None, "containerId": cid}) + "\n")
        time.sleep(5)


if __name__ == "__main__":
    main()

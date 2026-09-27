#!/usr/bin/env python3
"""Bounded local Linux probe for real rootless Docker cgroup enforcement.

Run as an ordinary user in a disposable systemd Linux VM. Requires the
official dockerd-rootless.sh, uidmap, slirp4netns, and static /bin/busybox.
Pass cgroupfs to reproduce the unsupported managed configuration; systemd
tests the supported user-service configuration. No existing daemon is used.
"""
import json
import os
from pathlib import Path
import subprocess
import sys
import tarfile
import tempfile
import time


def run(args, **kwargs):
    return subprocess.run(args, check=True, text=True, capture_output=True, timeout=20, **kwargs).stdout


def main():
    assert sys.platform == "linux" and os.geteuid() != 0
    assert len(sys.argv) == 2 and sys.argv[1] in ("cgroupfs", "systemd")
    driver = sys.argv[1]
    unit = f"chariox-cgroup-drill-{os.getpid()}"
    runtime = f"/run/user/{os.getuid()}/{unit}"
    start = time.monotonic()
    with tempfile.TemporaryDirectory(prefix="chariox-cgroup-state-") as root:
        docker = ["docker", "--host", f"unix://{runtime}/docker.sock"]
        # No provider profile, host Docker configuration, or account data.
        client_env = {"PATH": "/usr/bin:/bin:/usr/sbin:/sbin", "HOME": root, "DOCKER_CONFIG": root}
        def cli(*args):
            return run([*docker, *args], env=client_env)
        try:
            run(["systemd-run", "--user", "--unit", unit, "--collect",
                 "-p", "Delegate=yes", "-p", "MemoryMax=384M", "-p", "CPUQuota=50%",
                 "-p", "RuntimeMaxSec=90", "-p", "TimeoutStopSec=5", "-p", "KillMode=mixed",
                 "-p", f"RuntimeDirectory={unit}", "-p", "RuntimeDirectoryMode=0700",
                 "-p", f"WorkingDirectory={root}",
                 "--setenv", f"HOME={root}", "--setenv", f"XDG_RUNTIME_DIR={runtime}",
                 "--setenv", f"DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/{os.getuid()}/bus",
                 "/usr/bin/dockerd-rootless.sh", "--rootless",
                 "--host", f"unix://{runtime}/docker.sock", "--data-root", f"{root}/data",
                 "--exec-root", f"{root}/exec", "--pidfile", f"{runtime}/docker.pid",
                 "--storage-driver=vfs", "--iptables=false", "--bridge=none",
                 "--exec-opt", f"native.cgroupdriver={driver}"])
            info = None
            for _ in range(60):
                try:
                    info = json.loads(cli("info", "--format", "{{json .}}"))
                    break
                except subprocess.CalledProcessError:
                    if run(["systemctl", "--user", "show", f"{unit}.service", "-p", "ActiveState", "--value"]).strip() not in ("active", "activating"):
                        raise RuntimeError("disposable daemon exited before readiness")
                    time.sleep(0.25)
            assert info is not None, "disposable daemon readiness timeout"
            capability = {key: info.get(key) for key in
                          ("CgroupDriver", "CgroupVersion", "MemoryLimit", "PidsLimit", "CpuCfsQuota")}
            print(json.dumps(capability), flush=True)
            assert capability == {"CgroupDriver": "systemd", "CgroupVersion": "2",
                                  "MemoryLimit": True, "PidsLimit": True, "CpuCfsQuota": True}, "rootless resource controls are not enforced"
            image = f"{unit}:local"
            archive = Path(root) / "busybox.tar"
            with tarfile.open(archive, "w") as tar:
                tar.add("/bin/busybox", arcname="bin/busybox", recursive=False)
            cli("import", str(archive), image)
            values = cli("run", "--rm", "--network=none", "--memory=64m", "--cpus=0.2", "--pids-limit=32",
                         image, "/bin/busybox", "sh", "-c",
                         "/bin/busybox cat /sys/fs/cgroup/memory.max /sys/fs/cgroup/cpu.max /sys/fs/cgroup/pids.max")
            assert values.splitlines() == ["67108864", "20000 100000", "32"], values
            print("kernel cgroup files confirm memory=64MiB, cpu=0.2, pids=32", flush=True)
        finally:
            subprocess.run(["systemctl", "--user", "stop", f"{unit}.service"], capture_output=True, timeout=15)
            state = run(["systemctl", "--user", "show", f"{unit}.service", "-p", "ActiveState", "--value"]).strip()
            assert state in ("inactive", "failed"), f"owned service did not stop: {state}"
            print(json.dumps({"unit": unit, "finalState": state,
                              "elapsedSeconds": round(time.monotonic() - start, 3)}), flush=True)
    assert not Path(root).exists(), "disposable Docker state was not removed"
    assert not Path(runtime).exists(), "disposable runtime directory was not removed"
    print("owned daemon, container, image and disposable state removed", flush=True)


if __name__ == "__main__":
    main()

#!/usr/bin/env python3
"""Exercise shipped service lifecycle on a disposable systemd Linux VM.

Run as root with the name of an existing non-root user with a live user bus,
delegated cgroup v2 controllers and rootless Docker prerequisites. All unit
names, paths and Docker state are private to this invocation. No images are
pulled and no existing Docker service is changed.
"""
import json
import os
from pathlib import Path
import pwd
import shutil
import subprocess
import sys
import tarfile
import tempfile
import time


def run(args, **kwargs):
    return subprocess.run(args, check=True, text=True, capture_output=True, timeout=kwargs.pop("timeout", 20), **kwargs).stdout.strip()


def main():
    assert sys.platform == "linux" and os.geteuid() == 0 and len(sys.argv) == 2
    user = pwd.getpwnam(sys.argv[1])
    assert user.pw_uid != 0
    repo = Path(__file__).resolve().parent.parent
    context = repo / "apps/kernel/slice-linux-docker"
    name = f"chariox-lifecycle-drill-{os.getpid()}"
    unit = name + ".service"
    engine = name + "-engine.service"
    runtime = Path("/run") / name
    state = Path("/var/lib") / name
    system_unit = Path("/run/systemd/system") / unit
    user_units = Path(f"/run/user/{user.pw_uid}/systemd/user")
    user_unit = user_units / engine
    env = {"PATH": "/usr/bin:/bin:/usr/sbin:/sbin", "HOME": str(state / "home"),
           "XDG_RUNTIME_DIR": f"/run/user/{user.pw_uid}",
           "DBUS_SESSION_BUS_ADDRESS": f"unix:path=/run/user/{user.pw_uid}/bus"}
    prefix = ["setpriv", f"--reuid={user.pw_uid}", f"--regid={user.pw_gid}", "--clear-groups"]
    def owner(*args):
        return run([*prefix, *args], env=env)
    def cli(*args):
        return owner("docker", "--host", f"unix://{runtime}/docker.sock", *args)
    def wait_ready():
        for _ in range(80):
            try:
                info = json.loads(cli("info", "--format", "{{json .}}"))
                assert all(info[k] for k in ("MemoryLimit", "CpuCfsQuota", "PidsLimit")), info
                assert info["CgroupDriver"] == "systemd" and info["CgroupVersion"] == "2"
                return
            except subprocess.CalledProcessError:
                time.sleep(0.25)
        raise AssertionError("managed adapter did not bring up a resource-enforcing daemon")
    def assert_stopped():
        assert owner("systemctl", "--user", "show", engine, "-p", "ActiveState", "--value") in ("inactive", "failed")
        assert not runtime.exists(), "adapter left its runtime directory behind"

    # The real adapter uses PrivateTmp, so fixture executables cannot live in /tmp.
    with tempfile.TemporaryDirectory(prefix="chariox-lifecycle-source-", dir="/run") as scratch:
        scratch_path = Path(scratch)
        scratch_path.chmod(0o755)
        helper = scratch_path / "managed-rootless-service.sh"
        helper.write_text((context / helper.name).read_text().replace("/run/chariox-docker", str(runtime))
                          .replace("chariox-docker", user.pw_name).replace("chariox-rootless-engine.service", engine))
        helper.chmod(0o755)
        adapter_text = (repo / "deploy/managed-kernel/chariox-rootless-docker.service").read_text()
        adapter_text = adapter_text.replace("/usr/lib/chariox/slice-build-context/apps/kernel/slice-linux-docker/managed-rootless-service.sh", str(helper))
        adapter_text = adapter_text.replace("User=chariox-docker", f"User={user.pw_name}").replace("Group=chariox-docker", f"Group={user.pw_gid}")
        adapter_text = adapter_text.replace("/var/lib/chariox-docker", str(state)).replace("/run/chariox-docker", str(runtime))
        adapter_text = adapter_text.replace("Directory=chariox-docker", f"Directory={name}")
        # The fixture has no publication tree. Keep the other real hardening.
        adapter_text = adapter_text.replace(" /var/lib/chariox-slice-share/.broker-private", "").replace(" /var/lib/chariox-slice-share/slices/development", "")
        adapter_text = adapter_text.replace("[Install]", "RuntimeMaxSec=90\nMemoryMax=64M\nCPUQuota=25%\n\n[Install]")
        engine_text = (context / "chariox-rootless-engine.service").read_text()
        engine_text = engine_text.replace("/usr/share/docker.io/contrib/dockerd-rootless.sh", "/usr/bin/dockerd-rootless.sh --rootless --storage-driver=vfs --iptables=false --bridge=none")
        engine_text = engine_text.replace("/var/lib/chariox-docker", str(state)).replace("/run/chariox-docker", str(runtime))
        engine_text += "\nMemoryMax=384M\nCPUQuota=50%\nRuntimeMaxSec=90\n"
        assert not system_unit.exists() and not user_unit.exists() and not state.exists() and not runtime.exists()
        try:
            (state / "home").mkdir(parents=True, mode=0o700)
            os.chown(state, user.pw_uid, user.pw_gid)
            os.chown(state / "home", user.pw_uid, user.pw_gid)
            owner("mkdir", "-p", str(user_units))
            user_unit.write_text(engine_text)
            os.chown(user_unit, user.pw_uid, user.pw_gid)
            system_unit.write_text(adapter_text)
            run(["systemctl", "daemon-reload"])
            run(["systemctl", "start", unit], timeout=80)
            wait_ready()
            archive = state / "busybox.tar"
            with tarfile.open(archive, "w") as tar:
                tar.add("/bin/busybox", arcname="bin/busybox", recursive=False)
            os.chown(archive, user.pw_uid, user.pw_gid)
            image = name + ":local"
            cli("import", str(archive), image)
            def limits():
                values = cli("run", "--rm", "--network=none", "--memory=64m", "--cpus=0.2", "--pids-limit=32", image,
                             "/bin/busybox", "cat", "/sys/fs/cgroup/memory.max", "/sys/fs/cgroup/cpu.max", "/sys/fs/cgroup/pids.max")
                assert values.splitlines() == ["67108864", "20000 100000", "32"], values
            limits()
            print("managed adapter start: real memory/CPU/PID limits enforced", flush=True)
            old_pid = owner("systemctl", "--user", "show", engine, "-p", "MainPID", "--value")
            owner("systemctl", "--user", "kill", "--kill-whom=main", "--signal=SIGKILL", engine)
            for _ in range(80):
                new_pid = owner("systemctl", "--user", "show", engine, "-p", "MainPID", "--value")
                if new_pid not in (old_pid, "0"):
                    break
                time.sleep(0.25)
            assert new_pid not in (old_pid, "0"), "adapter did not restart failed engine"
            wait_ready()
            limits()
            print("engine SIGKILL: adapter recovered with enforced limits", flush=True)
            old_pid = run(["systemctl", "show", unit, "-p", "MainPID", "--value"])
            run(["systemctl", "kill", "--kill-whom=main", "--signal=SIGKILL", unit])
            for _ in range(80):
                new_pid = run(["systemctl", "show", unit, "-p", "MainPID", "--value"])
                if new_pid not in (old_pid, "0"):
                    break
                time.sleep(0.25)
            assert new_pid not in (old_pid, "0"), "adapter did not recover after its own crash"
            wait_ready()
            limits()
            print("adapter SIGKILL: cleanup and recovery retained enforced limits", flush=True)
            run(["systemctl", "stop", unit])
            assert_stopped()
            run(["systemctl", "start", unit])
            wait_ready()
            limits()
            run(["systemctl", "stop", unit])
            assert_stopped()
            print("explicit stop/start: daemon stopped, image preserved, limits enforced", flush=True)
            user_unit.write_text(engine_text.replace("native.cgroupdriver=systemd", "native.cgroupdriver=cgroupfs"))
            failed = subprocess.run(["systemctl", "start", unit], text=True, capture_output=True, timeout=20)
            assert failed.returncode != 0, "adapter admitted Docker without resource enforcement"
            run(["systemctl", "stop", unit])
            assert_stopped()
            print("unsupported cgroup driver: boot readiness rejected and daemon stopped", flush=True)
        finally:
            subprocess.run(["systemctl", "stop", unit], capture_output=True, timeout=80)
            owner("systemctl", "--user", "stop", engine)
            assert_stopped()
            # Inactive units can already have been garbage-collected by systemd.
            subprocess.run(["systemctl", "reset-failed", unit], capture_output=True, timeout=10)
            subprocess.run([*prefix, "systemctl", "--user", "reset-failed", engine], env=env, capture_output=True, timeout=10)
            system_unit.unlink(missing_ok=True)
            user_unit.unlink(missing_ok=True)
            run(["systemctl", "daemon-reload"])
            owner("systemctl", "--user", "daemon-reload")
            shutil.rmtree(state)
    print("all owned units, daemon state, image and fixture files removed", flush=True)


if __name__ == "__main__":
    main()

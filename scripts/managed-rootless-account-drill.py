#!/usr/bin/env python3
"""Fresh nologin-principal acceptance on a disposable systemd Linux VM.

Run as root. Creates only a temporary service account, its subordinate-ID
allocation, scoped user-manager drop-in and linger entry. Removes all of them
after running the shipped-unit lifecycle drill. Refuses an overlapping ID range.
"""
import os
from pathlib import Path
import pwd
import shutil
import subprocess
import sys
import tempfile
import time


def run(args, **kwargs):
    return subprocess.run(args, check=True, text=True, capture_output=True,
                          timeout=kwargs.pop("timeout", 20), **kwargs).stdout.strip()


def main():
    assert sys.platform == "linux" and os.geteuid() == 0 and len(sys.argv) == 1
    repo = Path(__file__).resolve().parent.parent
    name = f"chariox-cgroup-{os.getpid()}"
    try:
        pwd.getpwnam(name)
        raise RuntimeError("fixture service account already exists")
    except KeyError:
        pass
    start, count = 231072, 65536
    for path in ("/etc/subuid", "/etc/subgid"):
        for line in Path(path).read_text().splitlines():
            if not line.strip() or line.startswith("#"):
                continue
            _, existing_start, existing_count = line.split(":")
            assert start + count <= int(existing_start) or int(existing_start) + int(existing_count) <= start, "fixture subordinate IDs overlap an existing allocation"
    assert Path("/usr/bin/dockerd-rootless.sh").is_file()
    assert Path("/bin/busybox").is_file()
    created = False
    manager = None
    dropin = None
    policy_created = False
    manager_owned = False
    with tempfile.TemporaryDirectory(prefix="chariox-account-drill-", dir="/var/lib") as home:
        try:
            run(["useradd", "--system", "--user-group", "--no-create-home",
                 "--home-dir", home, "--shell", "/usr/sbin/nologin", name])
            created = True
            user = pwd.getpwnam(name)
            assert 0 < user.pw_uid < 1000 and user.pw_shell == "/usr/sbin/nologin"
            os.chown(home, user.pw_uid, user.pw_gid)
            run(["usermod", "--add-subuids", f"{start}-{start + count - 1}",
                 "--add-subgids", f"{start}-{start + count - 1}", name])
            manager = f"user@{user.pw_uid}.service"
            dropin = Path("/run/systemd/system") / f"{manager}.d"
            assert not dropin.exists(), "fixture user-manager policy already exists"
            dropin.mkdir()
            policy_created = True
            policy = (repo / "apps/kernel/slice-linux-docker/chariox-rootless-user-manager.conf").read_text()
            # Bound the whole fixture user tree, including container scopes.
            (dropin / "50-chariox-docker.conf").write_text(policy + "\nMemoryMax=512M\nCPUQuota=75%\nTasksMax=256\n")
            run(["systemctl", "daemon-reload"])
            assert run(["systemctl", "show", manager, "-p", "ActiveState", "--value"]) == "inactive", "fixture UID already has an active user manager"
            manager_owned = True
            run(["loginctl", "enable-linger", name])
            run(["systemctl", "start", manager])
            for _ in range(40):
                if Path(f"/run/user/{user.pw_uid}/bus").is_socket():
                    break
                time.sleep(0.25)
            assert Path(f"/run/user/{user.pw_uid}/bus").is_socket(), "fresh user manager did not activate the session bus"
            controllers = run(["systemctl", "show", manager, "-p", "DelegateControllers", "--value"]).split()
            assert {"cpu", "cpuset", "io", "memory", "pids"} <= set(controllers), controllers
            print(f"fresh nologin user {name}: session bus and scoped controllers ready", flush=True)
            result = subprocess.run([sys.executable, str(repo / "scripts/managed-rootless-lifecycle-drill.py"), name], timeout=180)
            assert result.returncode == 0, "fresh-account lifecycle drill failed"
        finally:
            if created:
                if manager_owned:
                    run(["loginctl", "disable-linger", name])
                    run(["systemctl", "stop", manager], timeout=80)
                    assert run(["systemctl", "show", manager, "-p", "ActiveState", "--value"]) == "inactive"
                    run(["systemctl", "stop", f"user-runtime-dir@{user.pw_uid}.service"])
                run(["userdel", name])
                if policy_created:
                    shutil.rmtree(dropin)
                run(["systemctl", "daemon-reload"])
                for path in ("/etc/subuid", "/etc/subgid"):
                    assert not any(line.startswith(name + ":") for line in Path(path).read_text().splitlines())
                assert not (Path("/var/lib/systemd/linger") / name).exists()
                assert not Path(f"/run/user/{user.pw_uid}").exists()
    print("temporary account, subordinate IDs, linger entry, policy and home removed", flush=True)


if __name__ == "__main__":
    main()

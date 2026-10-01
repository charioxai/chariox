#!/usr/bin/env python3
import base64
import ctypes
import grp
import importlib.util
import json
import os
import pathlib
import pwd
import select
import signal
import socket
import stat
import subprocess
import sys
import tempfile
import time
import types
import unittest
from unittest.mock import patch

sys.dont_write_bytecode = True
SOURCE = pathlib.Path(__file__).with_name("managed-extension-build.py")
spec = importlib.util.spec_from_file_location("extension", SOURCE)
extension = importlib.util.module_from_spec(spec)
spec.loader.exec_module(extension)


def wire_environment(values):
    return {base64.b64encode(os.fsencode(name)).decode(): base64.b64encode(os.fsencode(value)).decode()
            for name, value in values.items()}


def request():
    return dict(phase="check", callerPid=os.getpid(), image="chariox-slice-extension:creation-a", policy="auto",
                environment=wire_environment({"HOME": "/tmp", "PATH": "/usr/bin:/bin"}), context=None, dockerfile=None, basename=None)


class RequestTests(unittest.TestCase):
    def test_cache_check_does_not_need_source_paths(self):
        self.assertEqual(extension.validate_request(request())["phase"], "check")

    def test_base_image_and_invalid_environment_are_rejected(self):
        for key, value in [("image", extension.BASE_IMAGE), ("image", "../unowned:image"), ("policy", "remote")]:
            payload = request()
            payload[key] = value
            with self.assertRaises(ValueError):
                extension.validate_request(payload)
        for environment in ({"bad=field": "x"}, {"good": "x\0y"}):
            payload = request()
            payload["environment"].update(wire_environment(environment))
            with self.assertRaises(ValueError):
                extension.validate_request(payload)
        payload = request()
        payload["environment"].update(wire_environment(dict(AWS_PROFILE="synthetic-profile", ARBITRARY_HELPER_SETTING="synthetic-setting",
                                      PYTHONPATH="/synthetic", DOCKER_CONTEXT="foreign", BASH_ENV="/synthetic")))
        extension.validate_request(payload)

    def test_custom_image_references_and_base_alias_protection(self):
        for image in ("my-slice:Tools_1", "registry.example:5000/team/tool:tag", "[::1]:5000/team/tool:tag",
                      "team/tool@sha256:" + "a" * 64, "DOCKER.IO/library/" + extension.BASE_IMAGE):
            extension.validate_request(dict(request(), image=image))
        for image in ("docker.io/library/" + extension.BASE_IMAGE, "index.docker.io/library/" + extension.BASE_IMAGE,
                      "library/" + extension.BASE_IMAGE, "--flag", "/tmp/image", "image\nflag"):
            with self.assertRaises(ValueError):
                extension.validate_request(dict(request(), image=image))


    def test_native_byte_and_large_environment_survive_wire_and_exec(self):
        settings = {b"HOME": b"/tmp", b"PATH": b"/usr/bin:/bin", b"BYTE_HELPER_SETTING": b"\xff"}
        settings.update({f"LARGE_HELPER_{n}".encode(): b"x" * 40960 for n in range(8)})
        native = subprocess.run([sys.executable, "-I", "-S", "-B", "-c",
                                 "import os;assert os.environb[b'BYTE_HELPER_SETTING']==bytes([255]);assert len(os.environb[b'LARGE_HELPER_7'])==40960"],
                                env=settings, stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=5)
        self.assertEqual(native.returncode, 0)
        wire = {base64.b64encode(name).decode(): base64.b64encode(value).decode() for name, value in settings.items()}
        self.assertGreater(len(json.dumps(wire)), 256 * 1024)
        decoded = extension.decode_environment(wire)
        self.assertEqual(os.fsencode(decoded["BYTE_HELPER_SETTING"]), bytes([255]))
        self.assertEqual(len(decoded["LARGE_HELPER_7"]), 40960)
    def test_build_requires_exact_fd_identity_and_safe_basename(self):
        payload = request()
        payload.update(phase="build", context=dict(fd=3, device=1, inode=2),
                       dockerfile=dict(fd=4, device=1, inode=3), basename="Customfile")
        extension.validate_request(payload)
        for name in ("../Dockerfile", ".", "", "a/b", "x\0y"):
            bad = dict(payload, basename=name)
            with self.assertRaises(ValueError):
                extension.validate_request(bad)
        for field in ("context", "dockerfile"):
            bad = dict(payload)
            bad[field] = dict(payload[field], unexpected=1)
            with self.assertRaises(ValueError):
                extension.validate_request(bad)


@unittest.skipUnless(sys.platform == "linux", "Linux descriptor ownership")
class DescriptorTests(unittest.TestCase):
    def test_reopened_descriptor_pins_followed_symlink_target_and_identity(self):
        with tempfile.TemporaryDirectory(prefix="chariox-extension-descriptor-") as scratch:
            root = pathlib.Path(scratch)
            target = root / "instructions"
            target.write_text("original")
            link = root / "Customfile"
            link.symlink_to("instructions")
            original = os.open(link, os.O_RDONLY)
            procfd = os.open(f"/proc/{os.getpid()}", os.O_PATH | os.O_DIRECTORY)
            pidfd = os.pidfd_open(os.getpid())
            opened = None
            try:
                info = os.fstat(original)
                identity = dict(fd=original, device=info.st_dev, inode=info.st_ino)
                target.rename(root / "pinned-instructions")
                target.write_text("replacement")
                opened = extension.open_source(procfd, pidfd, identity, "file")
                self.assertEqual(os.read(opened, 100), b"original")
                with self.assertRaises(ValueError):
                    extension.open_source(procfd, pidfd, dict(identity, inode=info.st_ino + 1), "file")
            finally:
                for fd in (opened, original, procfd, pidfd):
                    if fd is not None:
                        os.close(fd)

    def test_direct_invoker_rejects_nominated_unrelated_same_uid_process(self):
        child = subprocess.Popen([sys.executable, "-c", "import time;time.sleep(10)"])
        try:
            with self.assertRaisesRegex(ValueError, "direct sudo invoker"):
                extension.pin_invoker(child.pid, os.getuid())
        finally:
            child.terminate()
            child.wait(timeout=5)


@unittest.skipUnless(sys.platform == "linux" and os.geteuid() == 0 and os.environ.get("CHARIOX_RUN_PRIVILEGED_MOUNT_TESTS") == "1",
                     "explicitly enabled root namespace fixture")
class NamespaceTests(unittest.TestCase):
    def test_credential_helper_retains_existing_sudo_authority_after_uid_drop(self):
        user = pwd.getpwnam("nobody")
        with tempfile.TemporaryDirectory(prefix="chariox-extension-sudo-", dir="/run") as scratch:
            root = pathlib.Path(scratch)
            root.chmod(0o711)
            policy = root / "sudoers"
            timestamps = root / "timestamps"
            timestamps.mkdir(mode=0o700)
            policy.write_text("Defaults !requiretty\nDefaults !use_pty\nDefaults !syslog\n"
                              "Defaults !pam_session\nDefaults !pam_setcred\nDefaults timestamp_timeout=0\n" +
                              'Defaults timestampdir="' + str(timestamps) + '"\n' +
                              'Defaults logfile="' + str(root / "sudo.log") + '"\n' +
                              user.pw_name + " ALL=(root) NOPASSWD: /usr/bin/id -u\n")
            policy.chmod(0o440)
            home = root / "home"
            home.mkdir(mode=0o700)
            os.chown(home, user.pw_uid, user.pw_gid)
            credential = root / "docker-credential-synthetic"
            credential.write_text("#!/bin/sh\nexec /usr/bin/sudo -n -- /usr/bin/id -u\n")
            credential.chmod(0o755)
            script = root / "probe.py"
            script.write_text("""
import importlib.util,json,os,pathlib,subprocess,sys
sys.dont_write_bytecode=True
source,root,uid,gid,mode=sys.argv[1:];root=pathlib.Path(root);uid=int(uid);gid=int(gid)
spec=importlib.util.spec_from_file_location("extension",source);m=importlib.util.module_from_spec(spec);spec.loader.exec_module(m)
m.mount("--make-rprivate","/");m.mount("--bind",root/"sudoers","/etc/sudoers")
if mode=="helper":m.drop_user(uid,gid,[])
else:os.setgroups([]);os.setgid(gid);os.setuid(uid)
status=dict(line.split(":",1) for line in pathlib.Path("/proc/self/status").read_text().splitlines() if ":" in line)
p=subprocess.run([str(root/"docker-credential-synthetic")],env={"HOME":str(root/"home"),"PATH":"/usr/bin:/bin"},capture_output=True,text=True,timeout=5)
print(json.dumps({"exit":p.returncode,"rootId":p.stdout.strip()=="0","noNewPrivileges":int(status["NoNewPrivs"]),
                 "effectiveCapabilities":int(status["CapEff"],16),"permittedCapabilities":int(status["CapPrm"],16)}))
""")
            for mode in ("ordinary", "helper"):
                result = subprocess.run(["/usr/bin/unshare", "--mount", "--pid", "--fork", "--mount-proc", "--kill-child=KILL",
                                         sys.executable, "-I", "-S", "-B", str(script), str(SOURCE), str(root),
                                         str(user.pw_uid), str(user.pw_gid), mode],
                                        stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, timeout=10)
                self.assertEqual(result.returncode, 0, result.stderr)
                observed = json.loads(result.stdout)
                self.assertEqual(observed, dict(exit=0, rootId=True, noNewPrivileges=0,
                                               effectiveCapabilities=0, permittedCapabilities=0), mode)

    def test_socket_group_requires_protected_unambiguous_daemon_allocation(self):
        user = types.SimpleNamespace(pw_name="chariox-docker", pw_uid=23457, pw_gid=23457)
        with tempfile.TemporaryDirectory(prefix="chariox-extension-subgid-", dir="/run") as scratch:
            allocation = pathlib.Path(scratch) / "subgid"
            allocation.write_text("chariox-docker:231072:65536\nother:400000:65536\n")
            allocation.chmod(0o600)
            with patch.object(extension, "SUBGID", allocation, create=True):
                self.assertEqual(extension.daemon_socket_group(user, 23457), 23457)
                self.assertEqual(extension.daemon_socket_group(user, 231178), 231178)
                for content in ("other:231072:65536\n", "chariox-docker:231072:invalid\n",
                                "chariox-docker:231072:65536\nother:231178:1\n",
                                "chariox-docker:231072:65536\n23457:231178:1\n",
                                "chariox-docker:4294967294:2\n"):
                    allocation.write_text(content)
                    with self.assertRaises(ValueError):
                        extension.daemon_socket_group(user, 231178)
                allocation.write_text("chariox-docker:231072:65536\n")
                allocation.chmod(0o622)
                with self.assertRaises(ValueError):
                    extension.daemon_socket_group(user, 231178)
                allocation.chmod(0o600)
                os.chown(allocation, 23457, 23457)
                with self.assertRaises(ValueError):
                    extension.daemon_socket_group(user, 231178)
                os.chown(allocation, 0, 0)
                allocation.rename(allocation.with_name("real-subgid"))
                allocation.symlink_to("real-subgid")
                with self.assertRaises(ValueError):
                    extension.daemon_socket_group(user, 231178)

    def test_mapped_rootless_socket_keeps_actual_group_access_after_uid_drop(self):
        with tempfile.TemporaryDirectory(prefix="chariox-extension-socket-", dir="/run") as scratch:
            root = pathlib.Path(scratch)
            root.chmod(0o711)
            daemon = root / "daemon"
            daemon.mkdir(mode=0o755)
            os.chown(daemon, 23457, 23457)
            allocation = root / "subgid"
            allocation.write_text("chariox-docker:231072:65536\n")
            allocation.chmod(0o600)
            ready_r, ready_w = os.pipe()
            go_r, go_w = os.pipe()
            child = os.fork()
            if child == 0:
                try:
                    os.close(ready_r)
                    os.close(go_w)
                    extension.native_call("unshare", 0x10000000)  # CLONE_NEWUSER
                    os.write(ready_w, b"u")
                    assert os.read(go_r, 1) == b"m"
                    os.setresgid(0, 0, 0)
                    os.setresuid(0, 0, 0)
                    server = socket.socket(socket.AF_UNIX)
                    server.bind(str(daemon / "docker.sock"))
                    # Docker's default socket group lookup inside the rootless
                    # namespace can select a subordinate, not primary host GID.
                    os.chown(daemon / "docker.sock", 0, grp.getgrnam("docker").gr_gid)
                    os.chmod(daemon / "docker.sock", 0o660)
                    server.listen(1)
                    os.write(ready_w, b"s")
                    peer, _ = server.accept()
                    assert peer.recv(4) == b"ping"
                    peer.sendall(b"pong")
                    peer.close()
                    server.close()
                    os._exit(0)
                except BaseException:
                    os._exit(1)
            os.close(ready_w)
            os.close(go_r)
            pidfd = os.pidfd_open(child)
            try:
                self.assertTrue(select.select([ready_r], [], [], 5)[0])
                self.assertEqual(os.read(ready_r, 1), b"u")
                pathlib.Path(f"/proc/{child}/uid_map").write_text("0 23457 1\n1 131072 65536\n")
                pathlib.Path(f"/proc/{child}/setgroups").write_text("deny\n")
                pathlib.Path(f"/proc/{child}/gid_map").write_text("0 23457 1\n1 231072 65536\n")
                os.write(go_w, b"m")
                self.assertTrue(select.select([ready_r], [], [], 5)[0])
                self.assertEqual(os.read(ready_r, 1), b"s")
                path = daemon / "docker.sock"
                self.assertNotEqual(path.stat().st_gid, 23457)
                user = types.SimpleNamespace(pw_name="chariox-docker", pw_uid=23457, pw_gid=23457)
                with patch.object(extension, "SOCKET", path), patch.object(extension, "SUBGID", allocation, create=True), patch.object(extension.pwd, "getpwnam", return_value=user):
                    fd, actual_gid = extension.pin_socket()
                    try:
                        self.assertEqual(actual_gid, path.stat().st_gid)
                        result = extension.capture_build([sys.executable, "-I", "-S", "-B", "-c",
                            "import socket,sys;s=socket.socket(socket.AF_UNIX);s.connect(sys.argv[1]);s.sendall(b'ping');assert s.recv(4)==b'pong'",
                            str(path)], {"HOME": str(root)}, 23456, 23456, [actual_gid], timeout=5)
                        self.assertEqual(result, 0)
                    finally:
                        os.close(fd)
                    for mutation in ("foreign-owner", "other-access", "no-group-write"):
                        if mutation == "foreign-owner":
                            os.chown(path, 23456, actual_gid)
                        else:
                            os.chown(path, 23457, actual_gid)
                            path.chmod(0o666 if mutation == "other-access" else 0o640)
                        with self.assertRaises(ValueError):
                            extension.pin_socket()
                waited, status = os.waitpid(child, 0)
                self.assertEqual(waited, child)
                self.assertEqual(os.waitstatus_to_exitcode(status), 0)
            finally:
                try:
                    signal.pidfd_send_signal(pidfd, signal.SIGKILL)
                except ProcessLookupError:
                    pass
                try:
                    os.waitpid(child, 0)
                except ChildProcessError:
                    pass
                for fd in (pidfd, ready_r, go_w):
                    os.close(fd)

    def test_projection_keeps_0600_copy_symlink_and_pinned_dockerfile(self):
        with tempfile.TemporaryDirectory(prefix="chariox-extension-projection-") as scratch:
            root = pathlib.Path(scratch)
            os.chmod(root, 0o711)
            context = root / "context"
            context.mkdir(mode=0o700)
            os.chown(context, 23456, 23456)
            file = context / "Customfile"
            target = context / "instructions"
            target.write_text("FROM scratch\n")
            target.chmod(0o600)
            os.chown(target, 23456, 23456)
            file.symlink_to("instructions")
            sibling = context / "sibling"
            sibling.write_text("copy sibling")
            sibling.chmod(0o600)
            os.chown(sibling, 23456, 23456)
            (context / "link").symlink_to("sibling")
            (context / "nested").mkdir()
            contextfd = os.open(context, os.O_PATH | os.O_DIRECTORY)
            filefd = os.open(file, os.O_RDONLY)
            lease = root / "lease"
            lease.mkdir()
            ready = root / "ready"
            release = root / "release"
            script = root / "projection.py"
            script.write_text("""
import base64
import importlib.util, os, pathlib, sys, time
sys.dont_write_bytecode=True
spec=importlib.util.spec_from_file_location("extension",sys.argv[1]);m=importlib.util.module_from_spec(spec);spec.loader.exec_module(m)
lease=pathlib.Path(sys.argv[2])
m.native_call("unshare",0x00020000);m.mount("--make-rprivate","/")
nested=pathlib.Path(sys.argv[7])/"nested"
m.mount("-t","tmpfs","-o","mode=0755,nodev,nosuid","tmpfs",nested)
(nested/"mounted-content").write_text("mounted sibling")
os.chown(nested/"mounted-content",23456,23456)
os.chmod(nested/"mounted-content",0o600)
m.mount("-t","tmpfs","-o","mode=0711,nodev,nosuid","tmpfs",lease)
context,file=m.project_context(lease,int(sys.argv[3]),int(sys.argv[4]),"Customfile",23456,23456)
pathlib.Path(sys.argv[5]).touch()
while not pathlib.Path(sys.argv[6]).exists():time.sleep(.01)
m.drop_user(23456,23456,[])
assert file.read_text()=="FROM scratch\\n"
assert (context/"sibling").read_text()=="copy sibling"
assert (context/"nested"/"mounted-content").read_text()=="mounted sibling"
try:(context/"nested"/"mounted-content").write_text("mutation");raise AssertionError("nested mount writable")
except OSError:pass
assert (context/"sibling").stat().st_mode&0o777==0o600
assert (context/"link").is_symlink() and os.readlink(context/"link")=="sibling"
assert (context/"Customfile").is_symlink() and os.readlink(context/"Customfile")=="instructions"
print("projection retained original instructions and context")
""")
            process = subprocess.Popen([sys.executable, "-I", "-S", "-B", str(script), str(SOURCE), str(lease),
                                        str(contextfd), str(filefd), str(ready), str(release), str(context)],
                                       pass_fds=(contextfd, filefd), stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
            try:
                for _ in range(300):
                    if ready.exists() or process.poll() is not None:
                        break
                    time.sleep(.01)
                self.assertTrue(ready.exists(), process.stderr.read() if process.poll() is not None else "mount timeout")
                context.rename(root / "original")
                context.symlink_to(root / "outside")
                # A caller can replace the leaf in the original directory; the
                # selected instructions remain pinned separately from COPY.
                moved = root / "original"
                (moved / "instructions").rename(moved / "old-instructions")
                (moved / "instructions").write_text("replacement")
                os.chown(moved / "instructions", 23456, 23456)
                # Keep context's selected symlink unchanged.
                release.touch()
                out, err = process.communicate(timeout=10)
                self.assertEqual(process.returncode, 0, err)
                self.assertIn("retained", out)
                self.assertFalse(os.path.ismount(lease))
            finally:
                if process.poll() is None:
                    process.kill()
                    process.wait(timeout=5)
                os.close(contextfd)
                os.close(filefd)

    def test_pid_namespace_bounds_flood_and_escaped_pipe_writer(self):
        for code, expected in [
            ("import os;os.write(1,b'x'*65536)", 0),
            ("import os,time;pid=os.fork();"
             "os.setsid() if pid==0 else None;"
             "time.sleep(20) if pid==0 else None", 0),
        ]:
            with tempfile.TemporaryDirectory(prefix="chariox-extension-process-") as scratch:
                root = pathlib.Path(scratch)
                lease = root / "lease"
                lease.mkdir()
                script = root / "process.py"
                script.write_text("""
import base64
import importlib.util, os, pathlib, sys, time
sys.dont_write_bytecode=True
spec=importlib.util.spec_from_file_location("extension",sys.argv[1]);m=importlib.util.module_from_spec(spec);spec.loader.exec_module(m)
before=time.monotonic()
status=m.run_namespace(lambda:m.capture_build([sys.executable,"-c",sys.argv[3]],{"HOME":"/tmp","PATH":"/usr/bin:/bin"},0,0,[],timeout=3,max_output=1024),pathlib.Path(sys.argv[2]),timeout=5)
assert time.monotonic()-before<4
assert status==int(sys.argv[4]),status
assert not pathlib.Path(sys.argv[2]).exists()
""")
                result = subprocess.run([sys.executable, "-I", "-S", "-B", str(script), str(SOURCE), str(lease), code, str(expected)],
                                        stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, text=True, timeout=10)
                self.assertEqual(result.returncode, 0, result.stderr)

    def test_caller_exit_and_sigkill_settle_owned_namespace_children(self):
        for mode in ("caller", "kill"):
            with tempfile.TemporaryDirectory(prefix="chariox-extension-lifetime-") as scratch:
                root = pathlib.Path(scratch)
                leases = root / "leases"
                leases.mkdir(mode=0o711)
                started = root / "started"
                leasefile = root / "lease-path"
                caller = subprocess.Popen([sys.executable, "-c", "import time;time.sleep(30)"])
                script = root / "lifetime.py"
                script.write_text("""
import base64
import importlib.util, os, pathlib, sys
sys.dont_write_bytecode=True
spec=importlib.util.spec_from_file_location("extension",sys.argv[1]);m=importlib.util.module_from_spec(spec);spec.loader.exec_module(m)
lease=m.scratch_lease(pathlib.Path(sys.argv[2]));pathlib.Path(sys.argv[5]).write_text(str(lease))
callerfd=os.pidfd_open(int(sys.argv[3]))
code="import pathlib,time;pid=pathlib.Path('/proc/self/status').read_text().split('NSpid:')[1].split()[0];pathlib.Path("+repr(sys.argv[4])+").write_text(pid);time.sleep(30)"
status=m.run_namespace(lambda:m.capture_build([sys.executable,"-c",code],{"HOME":"/tmp","PATH":"/usr/bin:/bin"},0,0,[]),lease,caller_pidfd=callerfd)
assert status==130,status
""")
                worker = subprocess.Popen([sys.executable, "-I", "-S", "-B", str(script), str(SOURCE), str(leases),
                                           str(caller.pid), str(started), str(leasefile)], stdout=subprocess.PIPE, stderr=subprocess.PIPE)
                childfd = None
                try:
                    for _ in range(400):
                        if started.exists() or worker.poll() is not None:
                            break
                        time.sleep(.01)
                    self.assertTrue(started.exists(), "lifetime fixture did not start")
                    childfd = os.pidfd_open(int(started.read_text()))
                    if mode == "caller":
                        caller.terminate()
                        caller.wait(timeout=5)
                        out, err = worker.communicate(timeout=5)
                        self.assertEqual(worker.returncode, 0, err.decode())
                        self.assertFalse(pathlib.Path(leasefile.read_text()).exists())
                    else:
                        worker.kill()
                        worker.communicate(timeout=5)
                        oldlease = pathlib.Path(leasefile.read_text())
                        self.assertTrue(oldlease.exists())
                        self.assertEqual(list(oldlease.iterdir()), [])
                        active = leases / f"{os.getpid()}-{extension.process_start(os.getpid())}-{'d'*16}"
                        active.mkdir(mode=0o700)
                        newlease = extension.scratch_lease(leases)
                        self.assertFalse(oldlease.exists())
                        self.assertTrue(active.exists())
                        newlease.rmdir()
                    self.assertTrue(select.select([childfd], [], [], 5)[0], "owned build survived termination")
                finally:
                    if childfd is not None:
                        os.close(childfd)
                    for process in (worker, caller):
                        if process.poll() is None:
                            process.kill()
                        process.communicate(timeout=5)

    def test_new_scratch_modes_are_deterministic_under_restrictive_umask(self):
        with tempfile.TemporaryDirectory(prefix="chariox-extension-umask-") as scratch:
            root = pathlib.Path(scratch) / "leases"
            old = os.umask(0o777)
            try:
                original_mkdir = pathlib.Path.mkdir
                created = []
                def observed_mkdir(path, mode=0o777, *args, **kwargs):
                    result = original_mkdir(path, mode, *args, **kwargs)
                    # Observe the return from mkdir, before any later chmod.
                    created.append((path, stat.S_IMODE(path.stat().st_mode)))
                    return result
                with patch.object(pathlib.Path, "mkdir", observed_mkdir):
                    lease = extension.scratch_lease(root)
                self.assertEqual(created, [(root, 0o711), (lease, 0o700)])
            finally:
                os.umask(old)
            self.assertEqual(stat.S_IMODE(root.stat().st_mode), 0o711)
            self.assertEqual(stat.S_IMODE(lease.stat().st_mode), 0o700)
            lease.rmdir()


    def test_namespace_tmpdir_is_writable_after_uid_drop_with_restrictive_umask(self):
        with tempfile.TemporaryDirectory(prefix="chariox-extension-private-tmp-") as scratch:
            root = pathlib.Path(scratch)
            root.chmod(0o711)
            script = root / "fixture.py"
            script.write_text("""
import base64
import importlib.util, json, os, pathlib, socket, sys
sys.dont_write_bytecode=True
spec=importlib.util.spec_from_file_location("extension",sys.argv[1]);m=importlib.util.module_from_spec(spec);spec.loader.exec_module(m)
root=pathlib.Path(sys.argv[2])
workspace=root/"workspace";workspace.mkdir(mode=0o700);os.chown(workspace,23456,23456)
setting=workspace/"relative-setting";setting.write_text("synthetic-relative-value");setting.chmod(0o600);os.chown(setting,23456,23456)
os.chdir(workspace)
procfd=os.open("/proc/self",os.O_PATH|os.O_DIRECTORY);pidfd=os.pidfd_open(os.getpid())
cwd=m.pin_cwd(procfd,pidfd);os.chdir(root)
os.umask(0o777)
server=socket.socket(socket.AF_UNIX);server.bind(str(root/"daemon.sock"))
fd=os.open(root/"daemon.sock",os.O_PATH)
lease=m.scratch_lease(root/"leases")
payload=dict(phase="check",image="custom:tag",policy="auto",environment={"HOME":str(root),"PATH":"/usr/bin:/bin",
"AWS_PROFILE":"synthetic-profile","ARBITRARY_HELPER_SETTING":"synthetic-setting",
"DOCKER_CONTEXT":"foreign","DOCKER_TLS_VERIFY":"1","DOCKER_CERT_PATH":"/missing-caller-cert","BASH_ENV":"/missing-caller-hook",
"RELATIVE_HELPER_FILE":"relative-setting","BYTE_HELPER_SETTING":os.fsdecode(bytes([255])),
"LARGE_HELPER_SETTING":"x"*40960,
"CHARIOX_SLICE_BASE_IMAGE":"foreign:base","CHARIOX_SLICE_IMAGE_BUILD_CHECK_ONLY":"bad"})
payload["environment"]=m.decode_environment({m.base64.b64encode(os.fsencode(k)).decode():m.base64.b64encode(os.fsencode(v)).decode() for k,v in payload["environment"].items()})
try:
 status=m.run_namespace(lambda:m.namespace_build(payload,lease,fd,None,None,23456,23456,[],"sha256:"+"a"*64,root/"provisioner",caller_cwd_fd=cwd),lease)
 assert status==0,status
finally:os.close(fd);os.close(cwd);os.close(procfd);os.close(pidfd);server.close()
""")
            provisioner = root / "provisioner"
            provisioner.write_text('set -eu\npython3 -I -S -B -c \'import os;assert os.environb[b"BYTE_HELPER_SETTING"]==bytes([255]);assert len(os.environb[b"LARGE_HELPER_SETTING"])==40960\'\ntest "$AWS_PROFILE" = synthetic-profile\ntest "$ARBITRARY_HELPER_SETTING" = synthetic-setting\ntest -z "${DOCKER_CONTEXT-}"\ntest -z "${BASH_ENV-}"\ntest -z "${DOCKER_TLS_VERIFY-}"\ntest -z "${DOCKER_CERT_PATH-}"\ntest "$(cat "$RELATIVE_HELPER_FILE")" = synthetic-relative-value\ntest "$CHARIOX_SLICE_BASE_IMAGE" = chariox-slice-linux:0.1.0\nprintf writable > "$TMPDIR/probe"\ntest "$(stat -c %a "$TMPDIR")" = 700\nprintf writable\n')
            process = subprocess.run([sys.executable, "-I", "-S", "-B", str(script), str(SOURCE), str(root)],
                                     stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, timeout=10)
            self.assertEqual(process.returncode, 0, process.stderr)
            self.assertEqual(process.stdout.strip(), "writable")

    def test_saturated_stale_leases_reclaim_before_live_limit_and_keep_fail_closed_entries(self):
        with tempfile.TemporaryDirectory(prefix="chariox-extension-saturated-") as scratch:
            top = pathlib.Path(scratch)
            stale_root = top / "stale"
            extension.mkdir_exact(stale_root, 0o711)
            for index in range(256):
                extension.mkdir_exact(stale_root / f"999999999-1-{index:016x}", 0o700)
            lease = extension.scratch_lease(stale_root)
            self.assertEqual(list(stale_root.iterdir()), [lease])
            lease.rmdir()
            live_root = top / "live"
            extension.mkdir_exact(live_root, 0o711)
            for index in range(256):
                extension.mkdir_exact(live_root / f"{os.getpid()}-{extension.process_start(os.getpid())}-{index:016x}", 0o700)
            with self.assertRaisesRegex(ValueError, "lease limit"):
                extension.scratch_lease(live_root)
            self.assertEqual(len(list(live_root.iterdir())), 256)
            for problem in ("unknown", "unowned", "nonempty"):
                root = top / problem
                extension.mkdir_exact(root, 0o711)
                entry = root / ("unknown-name" if problem == "unknown" else f"999999999-1-{'e'*16}")
                extension.mkdir_exact(entry, 0o700)
                if problem == "unowned":
                    os.chown(entry, 23456, 23456)
                elif problem == "nonempty":
                    (entry / "retained").touch()
                with self.assertRaises(OSError if problem == "nonempty" else ValueError):
                    extension.scratch_lease(root)
                self.assertTrue(entry.exists())
                if problem == "nonempty":
                    self.assertTrue((entry / "retained").exists())

    def test_concurrent_helpers_can_reclaim_the_same_dead_empty_lease(self):
        with tempfile.TemporaryDirectory(prefix="chariox-extension-concurrent-") as scratch:
            top = pathlib.Path(scratch)
            leases = top / "leases"
            extension.mkdir_exact(leases, 0o711)
            extension.mkdir_exact(leases / f"999999999-1-{'f'*16}", 0o700)
            script = top / "reclaim.py"
            script.write_text("""
import importlib.util,pathlib,sys,time
sys.dont_write_bytecode=True
source,top,identity=sys.argv[1:];top=pathlib.Path(top)
spec=importlib.util.spec_from_file_location("extension",source);m=importlib.util.module_from_spec(spec);spec.loader.exec_module(m)
original=m.process_start
def process_start(pid):
 result=original(pid)
 if pid==999999999:
  assert result is None
  (top/("ready-"+identity)).touch()
  until=time.monotonic()+5
  while not (top/"go").exists():
   assert time.monotonic()<until,"reclaim barrier timeout"
   time.sleep(.01)
 return result
m.process_start=process_start
print(m.scratch_lease(top/"leases").name)
""")
            workers = [subprocess.Popen([sys.executable, "-I", "-S", "-B", str(script), str(SOURCE), str(top), str(index)],
                                        stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True) for index in range(2)]
            try:
                for _ in range(300):
                    if all((top / f"ready-{index}").exists() for index in range(2)):
                        break
                    if any(worker.poll() is not None for worker in workers):
                        break
                    time.sleep(.01)
                self.assertTrue(all((top / f"ready-{index}").exists() for index in range(2)), "helpers did not reach stale identity barrier")
                (top / "go").touch()
                outputs = [worker.communicate(timeout=10) for worker in workers]
                self.assertEqual([worker.returncode for worker in workers], [0, 0], outputs)
                self.assertEqual(len(list(leases.iterdir())), 2)
            finally:
                for worker in workers:
                    if worker.poll() is None:
                        worker.kill()
                        worker.wait(timeout=5)

    def test_stale_empty_lease_cleanup_preserves_live_and_refuses_nonempty(self):
        with tempfile.TemporaryDirectory(prefix="chariox-extension-leases-") as scratch:
            root = pathlib.Path(scratch)
            root.chmod(0o711)
            live = root / f"{os.getpid()}-{extension.process_start(os.getpid())}-{'a'*16}"
            live.mkdir(mode=0o700)
            stale = root / f"999999999-1-{'b'*16}"
            stale.mkdir(mode=0o700)
            lease = extension.scratch_lease(root)
            self.assertTrue(live.exists())
            self.assertFalse(stale.exists())
            lease.rmdir()
            bad = root / f"999999999-1-{'c'*16}"
            bad.mkdir(mode=0o700)
            (bad / "unowned").touch()
            with self.assertRaises(OSError):
                extension.scratch_lease(root)
            self.assertTrue((bad / "unowned").exists())


if __name__ == "__main__":
    unittest.main()

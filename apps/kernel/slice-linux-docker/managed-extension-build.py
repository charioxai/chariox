#!/usr/bin/env python3
"""Pinned user-context image builds for the full-sudo Path-1 host only."""
import base64
import binascii
import ctypes
import json
import os
import pathlib
import platform
import pwd
import re
import selectors
import select
import signal
import stat
import subprocess
import sys
import time

ROOT = pathlib.Path(__file__).resolve().parents[3]
PROVISIONER = ROOT / "apps/kernel/slice-linux-docker/provision-linux-docker-slice.sh"
POLICY = pathlib.Path("/etc/sudoers.d/90-chariox-path1")
MANIFEST = ROOT.parent / "release-manifest.json"
SOCKET = pathlib.Path("/run/chariox-docker/docker.sock")
SUBGID = pathlib.Path("/etc/subgid")
SCRATCH = pathlib.Path("/run/chariox-slice-extension")
BASE_IMAGE = "chariox-slice-linux:0.1.0"
IMAGE_GRAMMAR = json.loads(pathlib.Path(__file__).with_name("docker-image-reference.json").read_text())
MAX_OUTPUT = 4 * 1024 * 1024
# Linux exec caps argv+environment at 3/4 of the 8MiB _STK_LIM and
# each entry at 32 pages. Fivefold overhead covers base64 padding/JSON
# even for many tiny name/value pairs, plus the fixed request fields.
MAX_ENV_BYTES = 6 * 1024 * 1024
MAX_ENV_STRING_BYTES = 32 * os.sysconf("SC_PAGESIZE")
MAX_REQUEST_BYTES = 5 * MAX_ENV_BYTES + 4096
CLIENT_CONTROL_ENV = {
    "DOCKER_HOST", "DOCKER_CONTEXT", "DOCKER_TLS", "DOCKER_TLS_VERIFY", "DOCKER_CERT_PATH", "BUILDX_BUILDER", "BUILDX_HOST", "BUILDKIT_HOST",
    "BASH_ENV", "ENV", "SHELLOPTS", "BASHOPTS", "CDPATH", "GLOBIGNORE", "IFS",
}
LIBC = ctypes.CDLL(None, use_errno=True)


def require(condition, message):
    if not condition:
        raise ValueError(message)


def normalized_image_reference(value):
    if not isinstance(value, str):
        return None
    match = re.fullmatch(IMAGE_GRAMMAR["reference"], value)
    if not match:
        return None
    domain, path, tag, digest = match.groups()
    if not domain or not (domain == "localhost" or re.search(r"[.:\[]", domain) or re.search(r"[A-Z]", domain)):
        path = domain + "/" + path if domain else path
        domain = "docker.io"
    if domain == "index.docker.io":
        domain = "docker.io"
    if domain == "docker.io" and "/" not in path:
        path = "library/" + path
    if len(path) > IMAGE_GRAMMAR["maximumRepositoryPath"]:
        return None
    return domain + "/" + path + (":" + tag if tag else "" if digest else ":latest") + ("@" + digest if digest else "")


def decode_environment(wire):
    require(isinstance(wire, dict), "invalid Docker client environment fields")
    environment = {}
    used = 0
    for encoded_name, encoded_value in wire.items():
        require(isinstance(encoded_name, str) and isinstance(encoded_value, str), "invalid environment byte encoding")
        try:
            name = base64.b64decode(encoded_name, validate=True)
            value = base64.b64decode(encoded_value, validate=True)
        except (ValueError, binascii.Error):
            raise ValueError("invalid environment byte encoding") from None
        require(name and b"=" not in name and b"\0" not in name and b"\0" not in value, "invalid native environment entry")
        size = len(name) + len(value) + 2
        used += size
        require(size <= MAX_ENV_STRING_BYTES and used <= MAX_ENV_BYTES, "environment exceeds Linux native exec limits")
        decoded_name = os.fsdecode(name)
        require(decoded_name not in environment, "duplicate environment byte name")
        environment[decoded_name] = os.fsdecode(value)
    return environment


def validate_request(value):
    require(isinstance(value, dict), "invalid extension build request")
    require(set(value) == {"phase", "callerPid", "image", "policy", "environment", "context", "dockerfile", "basename"}, "invalid extension build fields")
    require(value["phase"] in {"check", "build"}, "invalid extension build phase")
    require(type(value["callerPid"]) is int and value["callerPid"] > 1, "invalid caller PID")
    image = normalized_image_reference(value["image"])
    require(image is not None, "invalid extension image reference")
    require(image != IMAGE_GRAMMAR["defaultBase"], "extension output must be distinct from the runtime base")
    require(value["policy"] in {"auto", "always", "never"}, "invalid extension build policy")
    value = dict(value)
    environment = decode_environment(value["environment"])
    value["environment"] = environment
    require(environment.get("HOME", "").startswith("/") and bool(environment.get("PATH")), "Docker client HOME and PATH are required")
    if value["phase"] == "check":
        require(all(value[k] is None for k in ("context", "dockerfile", "basename")), "cache check cannot nominate descriptors")
    else:
        name = value["basename"]
        require(isinstance(name, str) and name not in {"", ".", ".."} and "/" not in name and "\0" not in name and len(os.fsencode(name)) <= 255, "invalid Dockerfile basename")
        for key in ("context", "dockerfile"):
            identity = value[key]
            require(isinstance(identity, dict) and set(identity) == {"fd", "device", "inode"}, "invalid source descriptor identity")
            require(all(type(identity[k]) is int and identity[k] >= 0 for k in identity), "invalid source descriptor number")
            require(identity["fd"] >= 3, "source descriptor must not be stdio")
    return value


def trusted_path(path, owner=0):
    path = pathlib.Path(path)
    require(path.is_absolute() and path.resolve() == path, "trusted path contains an alias")
    for component in (path, *path.parents):
        info = component.lstat()
        require(not stat.S_ISLNK(info.st_mode) and info.st_uid == owner and not info.st_mode & 0o022, "trusted path is not root-owned and immutable")


def path1_user():
    user = pwd.getpwnam("chariox")
    require(user.pw_uid > 0, "invalid Path-1 user")
    trusted_path(POLICY)
    require(stat.S_ISREG(POLICY.lstat().st_mode) and POLICY.read_text() == "chariox ALL=(ALL) NOPASSWD: ALL\n", "full-sudo Path-1 policy is required")
    require(os.environ.get("SUDO_UID") == str(user.pw_uid), "extension helper requires the Path-1 sudo caller")
    trusted_path(PROVISIONER)
    require(str(ROOT).startswith("/usr/lib/chariox/releases/"), "helper must come from an installed signed release")
    return user


def process_info(pid):
    status = pathlib.Path(f"/proc/{pid}/status").read_text()
    fields = dict(line.split(":", 1) for line in status.splitlines() if ":" in line)
    return int(fields["PPid"].strip()), tuple(map(int, fields["Uid"].split()))


def pin_invoker(caller_pid, uid):
    pidfd = os.pidfd_open(caller_pid)
    procfd = os.open(f"/proc/{caller_pid}", os.O_PATH | os.O_DIRECTORY)
    try:
        require(not select.select([pidfd], [], [], 0)[0], "descriptor caller exited")
        # Only the direct invoking process behind actual sudo monitor wrappers
        # may supply FDs. Full-sudo remains the existing Path-1 authority model.
        sudo = os.stat("/usr/bin/sudo")
        candidate = os.getppid()
        for _ in range(4):
            parent, uids = process_info(candidate)
            executable = os.stat(f"/proc/{candidate}/exe")
            if (executable.st_dev, executable.st_ino) != (sudo.st_dev, sudo.st_ino):
                break
            require(uids[1] == 0, "untrusted sudo monitor")
            candidate = parent
        require(candidate == caller_pid, "descriptor caller is not the direct sudo invoker")
        _, uids = process_info(caller_pid)
        require(uids[0] == uid and uids[1] == uid, "descriptor caller UID does not match sudo")
        require(not select.select([pidfd], [], [], 0)[0], "descriptor caller exited")
        return pidfd, procfd
    except BaseException:
        os.close(procfd)
        os.close(pidfd)
        raise


def open_source(procfd, pidfd, identity, kind):
    require(not select.select([pidfd], [], [], 0)[0], "descriptor caller exited")
    flags = os.O_PATH | os.O_DIRECTORY if kind == "directory" else os.O_RDONLY | os.O_NONBLOCK
    descriptor = os.open("fd/" + str(identity["fd"]), flags, dir_fd=procfd)
    try:
        info = os.fstat(descriptor)
        expected_kind = stat.S_ISDIR if kind == "directory" else stat.S_ISREG
        require(expected_kind(info.st_mode), "source descriptor has the wrong type")
        require((info.st_dev, info.st_ino) == (identity["device"], identity["inode"]), "source descriptor identity changed")
        require(not select.select([pidfd], [], [], 0)[0], "descriptor caller exited")
        return descriptor
    except BaseException:
        os.close(descriptor)
        raise


def pin_cwd(procfd, pidfd):
    descriptor = os.open("cwd", os.O_PATH | os.O_DIRECTORY, dir_fd=procfd)
    if select.select([pidfd], [], [], 0)[0]:
        os.close(descriptor)
        raise ValueError("descriptor caller exited")
    return descriptor


def daemon_socket_group(user, gid):
    if gid == user.pw_gid:
        return gid
    # Rootless dockerd may chown its socket to a group inside its user
    # namespace. Only root-protected subordinate allocations for this daemon
    # may supply that mapped host GID; never grant an arbitrary host group.
    trusted_path(SUBGID)
    require(stat.S_ISREG(SUBGID.lstat().st_mode), "subordinate group allocation is not a regular file")
    allocations = []
    for line in SUBGID.read_text().splitlines():
        if not line or line.startswith("#"):
            continue
        fields = line.split(":")
        require(len(fields) == 3 and fields[0] and not any(c.isspace() for c in fields[0])
                and all(re.fullmatch(r"[0-9]+", number) for number in fields[1:]), "malformed subordinate group allocation")
        owner, start, count = fields[0], int(fields[1]), int(fields[2])
        require(start > 0 and count > 0 and start + count <= 4294967295, "invalid subordinate group range")
        allocations.append((owner, start, start + count))
    own = [allocation for allocation in allocations if allocation[0] in {user.pw_name, str(user.pw_uid)}]
    for allocation in own:
        require(not any(other is not allocation and max(allocation[1], other[1]) < min(allocation[2], other[2])
                        for other in allocations), "ambiguous daemon subordinate group allocation")
    require(sum(start <= gid < end for _, start, end in own) == 1, "Docker socket group is not allocated to its daemon")
    return gid


def pin_socket():
    user = pwd.getpwnam("chariox-docker")
    # The official lifecycle accesses this same external Unix socket without
    # entering daemon namespaces. Never change its ownership or permission bits.
    require(SOCKET.resolve() == SOCKET, "Docker socket path contains an alias")
    for path in (SOCKET.parent, *SOCKET.parent.parents):
        info = path.lstat()
        require(info.st_uid in {0, user.pw_uid} and not info.st_mode & 0o022 and not stat.S_ISLNK(info.st_mode), "Docker socket parent is not protected")
    fd = os.open(SOCKET, os.O_PATH | os.O_NOFOLLOW)
    info = os.fstat(fd)
    if not (stat.S_ISSOCK(info.st_mode) and info.st_uid == user.pw_uid and info.st_mode & 0o060 == 0o060 and not info.st_mode & 0o006):
        os.close(fd)
        raise ValueError("Docker socket lacks its protected daemon group access")
    try:
        return fd, daemon_socket_group(user, info.st_gid)
    except BaseException:
        os.close(fd)
        raise


def signed_digest():
    trusted_path(MANIFEST)
    artifacts = json.loads(MANIFEST.read_text()).get("artifacts", [])
    matching = [a for a in artifacts if a.get("name") == "chariox-slice-build-context" and a.get("path") == "/usr/lib/chariox/slice-build-context"]
    require(len(matching) == 1 and re.fullmatch(r"sha256:[a-f0-9]{64}", matching[0].get("sha256", "")), "signed release must identify the build context")
    return matching[0]["sha256"]


def native_call(name, argument):
    require(getattr(LIBC, name)(argument) == 0, f"{name} failed: errno {ctypes.get_errno()}")


def mount(*arguments):
    descriptors = tuple(int(str(a).rsplit("/", 1)[1]) for a in arguments if str(a).startswith("/proc/self/fd/"))
    subprocess.run(["/usr/bin/mount", *map(str, arguments)], pass_fds=descriptors, check=True, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


class MountAttributes(ctypes.Structure):
    _fields_ = [(name, ctypes.c_uint64) for name in ("attr_set", "attr_clr", "propagation", "userns_fd")]


def readonly_bind(source, destination, recursive=False):
    mount("--rbind" if recursive else "--bind", source, destination)
    if recursive:
        # Supported native releases target x86_64/aarch64 Linux >=5.12. A
        # recursive readonly projection retains nested mounts for Docker COPY.
        require(platform.machine() in {"x86_64", "aarch64"}, "unsupported recursive mount architecture")
        attributes = MountAttributes(1, 0, 0, 0)  # MOUNT_ATTR_RDONLY
        result = LIBC.syscall(ctypes.c_long(442), ctypes.c_int(-100), ctypes.c_char_p(os.fsencode(destination)),
                              ctypes.c_uint(0x8000), ctypes.byref(attributes), ctypes.sizeof(attributes))
        require(result == 0, f"recursive readonly mount failed: errno {ctypes.get_errno()}")
    else:
        mount("-o", "remount,bind,ro", destination)


def project_context(lease, context_fd, dockerfile_fd, basename, uid, gid):
    source, context, override, view = [lease / name for name in ("source", "context", "override", "dockerfile")]
    for path in (source, context, override, view):
        path.mkdir()
    readonly_bind(f"/proc/self/fd/{context_fd}", source, recursive=True)
    readonly_bind(source, context, recursive=True)
    # The overlay substitutes only the selected instructions. COPY uses the
    # separate original context, preserving symlink and metadata semantics.
    info = os.fstat(context_fd)
    os.chown(override, info.st_uid, info.st_gid)
    os.chmod(override, stat.S_IMODE(info.st_mode))
    os.utime(override, ns=(info.st_atime_ns, info.st_mtime_ns))
    (override / basename).touch(mode=0o600)
    readonly_bind(override, override)
    mount("-t", "overlay", "-o", f"ro,lowerdir={override}:{source}", "overlay", view)
    # Pin after mounting overlay: lower-layer nested mounts are not traversed.
    readonly_bind(f"/proc/self/fd/{dockerfile_fd}", view / basename)
    return context, view / basename


def process_start(pid):
    try:
        return pathlib.Path(f"/proc/{pid}/stat").read_text().rsplit(")", 1)[1].split()[19]
    except (FileNotFoundError, ProcessLookupError):
        return None


def mkdir_exact(path, mode):
    # Atomic mode at creation also survives SIGKILL between mkdir and return.
    # This single-threaded helper restores its process umask immediately.
    previous = os.umask(0)
    try:
        path.mkdir(mode=mode)
    finally:
        os.umask(previous)


def scratch_lease(root=SCRATCH):
    try:
        mkdir_exact(root, 0o711)
    except FileExistsError:
        pass
    info = root.lstat()
    require(stat.S_ISDIR(info.st_mode) and info.st_uid == 0 and stat.S_IMODE(info.st_mode) == 0o711, "extension scratch root is not protected")
    children = list(root.iterdir())
    require(len(children) < 256, "extension scratch lease limit reached")
    for path in children:
        match = re.fullmatch(r"([0-9]+)-([0-9]+)-([a-f0-9]{16})", path.name)
        require(match is not None, "unrecognized extension scratch entry")
        entry = path.lstat()
        require(stat.S_ISDIR(entry.st_mode) and entry.st_uid == 0 and stat.S_IMODE(entry.st_mode) == 0o700, "unowned extension scratch entry")
        if process_start(int(match[1])) != match[2]:
            # SIGKILL leaves an empty mountpoint only. Never recursively remove
            # caller context, mounted contents, or another live helper's lease.
            path.rmdir()
    name = f"{os.getpid()}-{process_start(os.getpid())}-{os.urandom(8).hex()}"
    lease = root / name
    mkdir_exact(lease, 0o700)
    return lease


def drop_user(uid, gid, groups):
    os.setgroups(groups)
    os.setgid(gid)
    os.setuid(uid)
    class Header(ctypes.Structure):
        _fields_ = [("version", ctypes.c_uint32), ("pid", ctypes.c_int)]
    class Data(ctypes.Structure):
        _fields_ = [("effective", ctypes.c_uint32), ("permitted", ctypes.c_uint32), ("inheritable", ctypes.c_uint32)]
    header = Header(0x20080522, 0)
    data = (Data * 2)()
    require(LIBC.capset(ctypes.byref(header), ctypes.byref(data)) == 0, "could not clear build capabilities")
    require(LIBC.prctl(38, 1, 0, 0, 0) == 0, "could not set no_new_privileges")


def capture_build(argv, environment, uid, gid, groups, timeout=None, max_output=MAX_OUTPUT, caller_cwd_fd=None):
    def prepare_child():
        drop_user(uid, gid, groups)
        if caller_cwd_fd is not None:
            os.fchdir(caller_cwd_fd)
            os.close(caller_cwd_fd)
    process = subprocess.Popen(argv, env=environment, cwd=None if caller_cwd_fd is not None else environment["HOME"],
        pass_fds=() if caller_cwd_fd is None else (caller_cwd_fd,),
        stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE, preexec_fn=prepare_child)
    streams = selectors.DefaultSelector()
    for stream in (process.stdout, process.stderr):
        os.set_blocking(stream.fileno(), False)
        streams.register(stream, selectors.EVENT_READ)
    deadline = None if timeout is None else time.monotonic() + timeout
    used = 0
    truncated = False
    finished_at = None
    try:
        while streams.get_map():
            status = process.poll()
            if status is not None and finished_at is None:
                finished_at = time.monotonic()
            if deadline is not None and time.monotonic() >= deadline:
                return 124
            # An escaped pipe writer cannot keep capture alive after the build
            # exits. Exiting our PID namespace init kills every remaining child.
            if finished_at is not None and time.monotonic() - finished_at >= 1:
                return status
            for key, _ in streams.select(0.1):
                data = os.read(key.fd, 8192)
                if not data:
                    streams.unregister(key.fileobj)
                    continue
                retained = data[:max(0, max_output - used)]
                used += len(retained)
                if retained:
                    os.write(1 if key.fileobj is process.stdout else 2, retained)
                if len(retained) < len(data) and not truncated:
                    os.write(2, b"extension build output truncated after capture limit\n")
                    truncated = True
        remaining = None if deadline is None else max(0, deadline - time.monotonic())
        try:
            return process.wait(timeout=remaining)
        except subprocess.TimeoutExpired:
            return 124
    finally:
        streams.close()
        process.stdout.close()
        process.stderr.close()


def namespace_build(request, lease, socket_fd, context_fd, dockerfile_fd, uid, gid, groups, digest, provisioner=PROVISIONER, caller_cwd_fd=None):
    native_call("unshare", 0x00020000)  # CLONE_NEWNS
    mount("--make-rprivate", "/")
    mount("-t", "tmpfs", "-o", "mode=0711,nodev,nosuid", "tmpfs", lease)
    socket = lease / "docker.sock"
    socket.touch()
    readonly_bind(f"/proc/self/fd/{socket_fd}", socket)
    temporary = lease / "tmp"
    mkdir_exact(temporary, 0o700)
    os.chown(temporary, uid, gid)
    # Credential helpers inherit caller settings only in the UID-dropped child.
    # Endpoint, shell startup and signed-script policy selectors stay owned here.
    environment = {name: value for name, value in request["environment"].items()
                   if name not in CLIENT_CONTROL_ENV and not name.startswith(("CHARIOX_SLICE_", "BASH_FUNC_"))}
    environment.update({
        "DOCKER_HOST": "unix://" + str(socket),
        "CHARIOX_SLICE_MANAGED_DOCKER_HOST": "unix://" + str(socket),
        "CHARIOX_SLICE_DOCKER_IMAGE": request["image"],
        "CHARIOX_SLICE_BASE_IMAGE": BASE_IMAGE,
        "CHARIOX_SLICE_BUILD_IMAGE": request["policy"],
        "CHARIOX_SLICE_BUILD_CONTEXT_DIGEST": digest,
        "TMPDIR": str(temporary),
    })
    if request["phase"] == "check":
        environment["CHARIOX_SLICE_IMAGE_BUILD_CHECK_ONLY"] = "1"
    else:
        context, dockerfile = project_context(lease, context_fd, dockerfile_fd, request["basename"], uid, gid)
        environment["CHARIOX_SLICE_EXTENSION_BUILD_CONTEXT"] = str(context)
        environment["CHARIOX_SLICE_EXTENSION_DOCKERFILE"] = str(dockerfile)
    return capture_build(["/bin/bash", str(provisioner), "build-image"], environment, uid, gid, groups, caller_cwd_fd=caller_cwd_fd)


def run_namespace(build, lease, timeout=None, caller_pidfd=None):
    # If the outer helper is SIGKILLed, its PID-namespace init dies too, and
    # Linux kills all namespace processes, including setsid descendants.
    parentfd = os.pidfd_open(os.getpid())
    native_call("unshare", 0x20000000)  # CLONE_NEWPID affects only subsequent fork.
    child = os.fork()
    if child == 0:
        try:
            require(LIBC.prctl(1, signal.SIGKILL, 0, 0, 0) == 0, "could not set parent-death signal")
            require(not select.select([parentfd], [], [], 0)[0], "extension supervisor exited")
            os.close(parentfd)
            os._exit(build())
        except BaseException as error:
            os.write(2, ("extension build failed: " + str(error) + "\n").encode())
            os._exit(1)
    os.close(parentfd)
    childfd = os.pidfd_open(child)
    deadline = None if timeout is None else time.monotonic() + timeout
    try:
        while deadline is None or time.monotonic() < deadline:
            if caller_pidfd is not None and select.select([caller_pidfd], [], [], 0)[0]:
                return 130
            waited, status = os.waitpid(child, os.WNOHANG)
            if waited:
                return os.waitstatus_to_exitcode(status)
            time.sleep(0.05)
        return 124
    finally:
        try:
            signal.pidfd_send_signal(childfd, signal.SIGKILL)
        except ProcessLookupError:
            pass
        os.close(childfd)
        try:
            os.waitpid(child, 0)
        except ChildProcessError:
            pass
        lease.rmdir()


def main():
    require(os.geteuid() == 0 and sys.platform == "linux" and len(sys.argv) == 1, "Linux root helper with stdin request required")
    # Killing the direct sudo child must also settle the privileged supervisor,
    # whose own parent-death-bound PID init owns every build descendant.
    parent = os.getppid()
    parentfd = os.pidfd_open(parent)
    try:
        require(LIBC.prctl(1, signal.SIGKILL, 0, 0, 0) == 0, "could not bind supervisor lifetime")
        require(os.getppid() == parent and not select.select([parentfd], [], [], 0)[0], "sudo monitor exited")
    finally:
        os.close(parentfd)
    user = path1_user()
    request_bytes = sys.stdin.buffer.read(MAX_REQUEST_BYTES + 1)
    require(len(request_bytes) <= MAX_REQUEST_BYTES, "extension request exceeded limit")
    request = validate_request(json.loads(request_bytes))
    descriptors = []
    lease = None
    try:
        pidfd, procfd = pin_invoker(request["callerPid"], user.pw_uid)
        descriptors += [pidfd, procfd]
        cwd_fd = pin_cwd(procfd, pidfd)
        descriptors.append(cwd_fd)
        context_fd = dockerfile_fd = None
        if request["phase"] == "build":
            context_fd = open_source(procfd, pidfd, request["context"], "directory")
            descriptors.append(context_fd)
            dockerfile_fd = open_source(procfd, pidfd, request["dockerfile"], "file")
            descriptors.append(dockerfile_fd)
        socket_fd, docker_gid = pin_socket()
        descriptors.append(socket_fd)
        digest = signed_digest()
        lease = scratch_lease()
        groups = sorted(set(os.getgrouplist(user.pw_name, user.pw_gid) + [docker_gid]))
        return run_namespace(lambda: namespace_build(request, lease, socket_fd, context_fd, dockerfile_fd, user.pw_uid, user.pw_gid, groups, digest, caller_cwd_fd=cwd_fd), lease, caller_pidfd=pidfd)
    finally:
        for descriptor in reversed(descriptors):
            os.close(descriptor)
        if lease is not None and lease.exists():
            lease.rmdir()


if __name__ == "__main__":
    try:
        signal.signal(signal.SIGTERM, lambda *_: (_ for _ in ()).throw(KeyboardInterrupt()))
        sys.exit(main())
    except (ValueError, OSError, subprocess.SubprocessError, KeyboardInterrupt) as error:
        print("managed extension build refused: " + str(error), file=sys.stderr)
        sys.exit(1)

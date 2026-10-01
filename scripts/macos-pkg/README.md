# Docker admission boot setup

The pkg carries the shared `deploy/local-linux/provision-docker-admission-locks.py`
at `/usr/local/libexec/chariox/provision-docker-admission-locks.py` and
`deploy/local-macos/dev.chariox.docker-admission-locks.plist` under
`/Library/LaunchDaemons/`. `postinstall` provisions synchronously before runtime
enrollment or kernel restart, then enables and bootstraps the root daemon. It
runs again at boot to recreate the two locks after `/tmp` clears.

The interpreter `/usr/bin/python3` must be available through Apple's Command
Line Tools. An unsafe legacy lock fails installation. Stop every kernel sharing
Docker before an administrator inspects and removes such a lock, then reinstall.
Never replace a live lock inode. `uninstall.sh` unloads the system daemon and
removes its files; it keeps the shared lock inodes until reboot.

Developer checkout setup and kernel recovery instructions are in the companion
PR based on #670, `deploy/local-linux/LOCAL_DOCKER_DEV.md`. This pkg change stacks
on #688 and uses the same provisioner and plist bytes as that companion change.

# Local Docker admission locks

Every kernel sharing a Docker engine uses the same two host locks:
`/tmp/chariox-docker-memory-admission.lock` and
`/tmp/chariox-docker-disk-admission.lock`. They must be empty root-owned regular
files with one link and no group/other writes. The provisioner uses mode 0444,
so ordinary kernels can open and lock them but cannot replace them in sticky
`/tmp`. It preserves an existing safe inode, including an active lock.

## macOS setup

The Chariox macOS pkg installs the root LaunchDaemon
`/Library/LaunchDaemons/dev.chariox.docker-admission-locks.plist` and runs the
shared provisioner before starting the kernel. At boot, launchd runs it again
because `/tmp` is volatile. macOS resolves `/tmp` to `/private/tmp`.
The daemon uses `/usr/bin/python3`; install the Apple Command Line Tools if that
interpreter is unavailable.

For a developer checkout, do this once as an administrator before starting the
Room or enrolling the developer App runtime:

```sh
sudo /usr/bin/python3 deploy/local-macos/install-docker-admission-locks.py
```

The developer runtime release receipt prints this command as
`provisionDockerAdmissionLocks` and prefixes its `enroll` command with it, so
runtime enrollment also installs boot provisioning. macOS App storage
uses the runtime's APFS implementation, so there is no Linux storage helper to
install here. The command installs the provisioner under
`/usr/local/libexec/chariox/`, provisions immediately, and enables and loads the
root boot daemon. No per-App privilege step is needed.

If a pkg is already installed, provision missing locks immediately with:

```sh
sudo /usr/bin/python3 /usr/local/libexec/chariox/provision-docker-admission-locks.py
```

Boot errors go to `/var/log/chariox-docker-admission-locks.log`.
`sudo launchctl print system/dev.chariox.docker-admission-locks` shows the daemon
and its last exit status. After an administrator repairs an unsafe lock, rerun
the setup command or reinstall the pkg to retry and restore boot provisioning.

## Unsafe locks from older kernels

The script refuses a user-owned lock, link, writable lock, or nonempty lock. It
never adopts or deletes it. First stop **all** kernels using this Docker engine,
including root, developer and other users' kernels. An administrator must then
inspect and remove only the unsafe admission lock files and rerun setup. Never
unlink a lock while any kernel can hold it: replacing its inode permits two
kernels to admit resources independently. Rebooting after installing the daemon
also clears legacy locks in volatile `/tmp`.

Uninstallation unloads the daemon and removes its program and plist. It keeps
the admission locks, since a developer kernel or another installation may still
hold their inodes; `/tmp` clears them on reboot.

## Linux

`deploy/local-linux/install-root.sh` and the managed-kernel installer deliver
`chariox-docker-admission-locks.service` and the same
`deploy/local-linux/provision-docker-admission-locks.py`. The service provisions
at boot before managed kernel activation. Manual repair follows the same rule
about stopping every kernel before removing an unsafe inode.

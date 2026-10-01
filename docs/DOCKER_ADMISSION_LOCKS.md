# Host-wide Docker admission locks

Memory and disk admission intentionally serialize every Docker context on a host.
Root and ordinary kernels use the same legacy files, independent of UID, HOME,
TMPDIR, rootless Docker sockets, and endpoint spelling:

- `/tmp/chariox-docker-memory-admission.lock`
- `/tmp/chariox-docker-disk-admission.lock`

Before using Docker-backed slices on standalone Linux, rootless Linux, or macOS,
run once from the installed source release:

```sh
sudo python3 deploy/local-linux/provision-docker-admission-locks.py
```

The Linux root installer and managed install/upgrade install the provisioner and
an ordered root oneshot service. It runs before basic boot services and is a
required dependency of the managed supervisor, before its namespace is created.
Provisioner and unit are included in the signed release context tree. Reboot or
a reset of volatile /tmp therefore recreates missing locks before managed start.
Standalone systems using only the manual command must run it again after /tmp
is reset; missing locks fail closed.
On macOS the script and opener resolve the standard `/tmp` alias to `/private/tmp`.
The files contain no data. They are root-owned mode 0444; kernels open read-only
and acquire an exclusive advisory lock. A root-owned sticky parent prevents an
ordinary user from unlinking or replacing a root-owned lock inode.

Provisioning never deletes, truncates, or replaces an existing file. A safe legacy
root-owned mode-0600 file is changed to 0444 in place. Older root kernels opening
read/write and new ordinary kernels opening read-only continue to lock the same
inode. Repeated provisioning preserves the inode and an already-held lock.

Missing provisioning fails with an actionable error. Foreign-owned legacy locks,
symlinks, hardlinks, writable files, and nonempty files are rejected. Stop every
kernel using the host Docker engine before an administrator repairs an unsafe
legacy file. Do not unlink a lock while any old or new kernel can hold it: that
would create two admission domains. There is no per-UID fallback.

A custom service with a private temporary mount namespace must bind these exact
host file inodes. The managed bootstrap unit declares both BindPaths. The root-owned
0444 file permissions keep ordinary callers read-only; the bind remains writable
for compatibility with legacy root callers opening the same inode read/write.
Stop and restart every old kernel in a private temporary namespace before
activating that unit change. Its old private inode was already separate from
host kernels; do not claim coexistence with the new host-bound inode. Provision
locks before starting the updated unit. No automatic namespace or per-UID
fallback is provided.

The Windows machine-wide admission mutexes are unchanged.

Rust tests use a process-owned disposable lock directory with the same metadata
checks and a test-only expected owner. Test subprocesses inherit that directory;
normal test exit removes only its two empty files and directory. Production
binaries have no test path or owner fallback.

# Managed rootless resource enforcement

The managed image's current rootless Docker unit runs as a system service with
`User=chariox-docker` and forces `native.cgroupdriver=cgroupfs`. The headed-slice
acceptance attempt on 2026-09-07 reported rootless mode without cgroups. This is
not evidence that the requested slice memory, CPU, or PID limits are enforced.

Docker supports rootless resource controls with cgroup v2 and a systemd user
service. A system-wide service with `User=` is not the supported substitute.
See [Docker's rootless service and resource documentation](https://docs.docker.com/engine/security/rootless/tips/).

## Reproduction

Run as an ordinary user in a disposable Linux VM with systemd, a running user
manager and session bus, rootless Docker prerequisites, and static
`/bin/busybox`. The user's delegated controllers must include CPU, memory and
PIDs. This probe does not install or alter delegation, change an existing
Docker daemon, pull images, or run a resource-exhaustion workload.

```sh
python3 scripts/managed-rootless-resource-drill.py cgroupfs
python3 scripts/managed-rootless-resource-drill.py systemd
```

The first command must fail the resource-capability assertion on the affected
configuration. The second starts its own systemd user service, verifies Docker's
reported support, and reads the kernel's cgroup files inside a tiny container:

- `memory.max` = 67108864
- `cpu.max` = 20000 100000
- `pids.max` = 32

Each probe caps its own daemon service at 384MiB and half a CPU, with a
90-second service deadline. It uses a private socket and temporary data root,
imports only the local static BusyBox executable, and removes the service,
container, image and state. No provider or host Docker configuration is used.

## Recorded result and remaining implementation

On the existing local ARM64 Ubuntu 24.04 test VM, Docker 29.2.1 with cgroup v2:

- `cgroupfs` reported driver `none` and memory/PID/CPU support `false`. The
  resource assertion failed as intended; its owned unit was stopped.
- `systemd` reported all required capabilities and the real container's cgroup
  files matched all three requested limits. Its owned unit was stopped.

This establishes a red-capable Linux drill and a working supported configuration.
It does **not** fix or accept the production managed image. The product change
must install and supervise a user service for the dedicated Docker principal,
delegate the required controllers only to that principal, and retain the
existing protected socket/state paths and separation from the kernel user.
Its system-service lifecycle adapter must propagate readiness, failure,
restart and stop without leaving a detached daemon. Update signed packaging,
image preparation and boot-time checks together. Verify actual cgroup files on
the fresh x86_64 Ubuntu 26.04 managed image before long workloads, including
after reboot and slice save/restore. Do not suppress the warning or substitute
unenforced Docker flags for this acceptance.

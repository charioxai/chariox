# Managed rootless resource enforcement

The affected managed image's rootless Docker unit runs as a system service with
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

## Shipped configuration and lifecycle regression

The signed release now includes a dedicated `chariox-rootless-engine.service`
user unit and a small system-service lifecycle adapter. The adapter retains
the protected socket and state directories, starts the dedicated user manager,
waits on the user engine, checks cgroup capabilities before reporting ready,
and stops the engine even when its own startup or main process fails.
Only the adapter owns restart policy. The user unit cannot independently
restart after the system service has released its runtime directory.

The installer enables lingering for `chariox-docker` and installs controller
delegation for that UID's `user@UID.service` only. Both the user unit and its
delegation file link into the signed release. Image preparation installs the
user session bus and checks real container cgroup files during the existing
mount-handle lifecycle drill. An unenforced resource configuration fails image
validation and boot readiness.

On a disposable systemd Linux VM with the prerequisites above, run:

```sh
sudo python3 scripts/managed-rootless-lifecycle-drill.py <ordinary-test-user>
```

This uses the shipped adapter, helper and engine unit with isolated fixture
names and paths. It retains adapter hardening, uses the installed official
Docker launcher with an offline VFS fixture, and caps the daemon at 384MiB and
half a CPU. It tests real container limits at start, after killing the engine,
after killing the adapter, and after explicit stop/start. It also switches the
fixture to the unsupported driver and requires boot readiness to fail, then
checks that the daemon stopped. All owned units, images and state are removed.

## Recorded result and remaining validation

On the existing local ARM64 Ubuntu 24.04 test VM, Docker 29.2.1 with cgroup v2:

- `cgroupfs` reported driver `none` and memory/PID/CPU support `false`. The
  resource assertion failed as intended; its owned unit was stopped.
- `systemd` reported all required capabilities and the real container's cgroup
  files matched all three requested limits. Its owned unit was stopped.

The shipped-unit lifecycle drill also passed all five scenarios on that local
VM. Its boot-check regression caught a Docker Go-template field-name mismatch
that static packaging tests did not catch; the corrected readiness command
was then exercised successfully, including rejection of the unsupported driver.

This is local implementation evidence, not production-image acceptance. The
fixture reuses an existing ordinary user with delegated controllers; it does
not prove fresh dedicated-account creation, image boot or reboot. Verify those
paths and actual container cgroup files on a fresh x86_64 Ubuntu 26.04 managed
image, then repeat after reboot and slice save/restore before long workloads.

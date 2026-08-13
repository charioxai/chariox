# Linux Slice via Lume

This package owns the macOS Lume provisioner for Linux slices. It is kept under
`apps/kernel` because the kernel owns slice runtime setup and lifecycle state.

Run:

```bash
bash apps/kernel/slice-linux-lume/scripts/provision-linux-lume-slice.sh
```

The provisioner:

- installs Lume if it is missing
- pulls the configured ARM64 Ubuntu VM image
- starts the VM headlessly
- SSHes into the VM using the configured image user
- installs Node.js, Codex CLI, OpenCode, Claude Code, Rust, and build tools
- copies `apps/kernel` and `apps/relay` into the VM
- builds `chariox-kernel` and `chariox-relay` inside the VM
- writes VM-local runtime helper scripts under `~/chariox-slice`
- optionally launches provider servers without credentials

Useful overrides:

```bash
CHARIOX_SLICE_NAME=chariox-slice-linux-lume \
CHARIOX_SLICE_START_PROVIDER_SERVERS=0 \
bash apps/kernel/slice-linux-lume/scripts/provision-linux-lume-slice.sh
```

Credentials are not imported by this Lume provisioner. Use provider-native login
inside the slice, or the kernel-owned slice auth import/remove commands for
backends that support explicit credential projection.

## Validation Scripts

The `.chariox` scripts in `scripts/` are smoke assets for provider-backed kernel
sessions from inside a slice. The Docker-backed production path remains in
`apps/kernel/slice-linux-docker`.

Example from a running Docker slice:

```bash
docker exec -u slice chariox-slice-linux \
  bash -lc 'CHARIOX_KERNEL_PORT=43119 node /workspace/apps/shell/dist/shell.js run /workspace/apps/kernel/slice-linux-lume/scripts/validate-kernel-session.chariox --workspace /workspace --worktree /workspace'
```

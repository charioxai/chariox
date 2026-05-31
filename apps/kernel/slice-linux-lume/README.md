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
- builds `arroba-kernel` and `arroba-relay` inside the VM
- writes VM-local runtime helper scripts under `~/arroba-slice`
- optionally launches provider servers without credentials

Useful overrides:

```bash
ARROBA_SLICE_NAME=arroba-slice-linux-lume \
ARROBA_SLICE_START_PROVIDER_SERVERS=0 \
bash apps/kernel/slice-linux-lume/scripts/provision-linux-lume-slice.sh
```

Credentials are not imported by this Lume provisioner. Use provider-native login
inside the slice, or the kernel-owned slice auth import/remove commands for
backends that support explicit credential projection.

## Validation Scripts

The `.arroba` scripts in `scripts/` are smoke assets for provider-backed kernel
sessions from inside a slice. The Docker-backed production path remains in
`apps/kernel/slice-linux-docker`.

Example from a running Docker slice:

```bash
docker exec -u slice arroba-slice-linux \
  bash -lc 'ARROBA_KERNEL_PORT=43119 node /workspace/apps/shell/dist/shell.js run /workspace/apps/kernel/slice-linux-lume/scripts/validate-kernel-session.arroba --workspace /workspace --worktree /workspace'
```

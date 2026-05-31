# Slice Spike

This experiment is intentionally outside Arroba product/runtime code. It validates whether a
local Mac can provision a Linux "slice" VM, install provider CLIs, build Arroba kernel/relay
inside the slice, and launch provider servers without manual setup.

## Linux on Mac via Lume

Run:

```bash
bash experiments/slice-linux-docker/scripts/provision-linux-lume-slice.sh
```

The script:

- installs Lume if it is missing
- pulls a prebuilt ARM64 Ubuntu VM image
- starts the VM headlessly
- SSHes into the VM using the prebuilt `lume` user
- installs Node.js, Codex CLI, OpenCode, Rust, and build tools
- copies only `apps/kernel` and `apps/relay` into the VM
- builds `arroba-kernel` and `arroba-relay` inside the VM
- writes VM-local runtime helper scripts under `~/arroba-slice`
- optionally launches Codex/OpenCode provider servers without credentials

Useful overrides:

```bash
ARROBA_SLICE_NAME=arroba-slice-linux \
ARROBA_SLICE_START_PROVIDER_SERVERS=0 \
bash experiments/slice-linux-docker/scripts/provision-linux-lume-slice.sh
```

Credentials are deliberately not imported by default. The next validation step is to compare
provider login/import paths once someone is available to authorize them:

- vault-backed API-key login
- provider-native login inside the slice
- explicit local provider config import

## Linux Provider Fallback via Docker Desktop

Lume remains the preferred Mac-local VM direction for this spike. On May 12, 2026, Lume `0.3.9`
installed successfully but `lume pull ubuntu-noble-vanilla:latest arroba-slice-linux`
failed before creating the VM because the image layers were skipped as unsupported. Until that
is resolved, use the Docker Desktop fallback to validate the Linux provider/kernel assumptions
without adding product code:

```bash
bash apps/kernel/slice-linux-docker/provision-linux-docker-slice.sh
```

This builds a local Debian image, installs Chromium/Codex/OpenCode, builds Arroba kernel/relay, starts a
persistent container with a provider-home volume, and launches provider servers on localhost.
It is a validation fallback, not the intended long-term isolation substrate.

Validate the GUI/browser-control layer without provider credentials:

```bash
bash apps/kernel/slice-linux-docker/provision-linux-docker-slice.sh validate-screen
```

This starts an Xvfb/Openbox desktop, launches headful Chromium, captures the screen, runs OCR with
bounding boxes, drives browser input through the local Chromium DevTools endpoint, validates clipboard
get/set, and prints the noVNC viewer URL.

Import provider-native login state from the host into the persistent slice home volume:

```bash
bash apps/kernel/slice-linux-docker/provision-linux-docker-slice.sh import-provider-auth
```

By default this copies:

- `~/.codex/auth.json` to `/home/slice/.codex/auth.json`
- `~/.local/share/opencode/auth.json` to `/home/slice/.local/share/opencode/auth.json`

The copied files are owned by the `slice` user with `0600` permissions, and any existing slice auth
file is backed up before replacement. Override paths with `ARROBA_SLICE_CODEX_AUTH` and
`ARROBA_SLICE_OPENCODE_AUTH`. To import during provisioning, set
`ARROBA_SLICE_IMPORT_PROVIDER_AUTH=1`.

Alternatively, log in directly inside the slice so the host provider account does not need to
change:

```bash
bash apps/kernel/slice-linux-docker/provision-linux-docker-slice.sh login-codex
bash apps/kernel/slice-linux-docker/provision-linux-docker-slice.sh login-opencode
```

Both ChatGPT account flows use OpenAI device authorization. The command prints a URL and one-time
code; authorize it in any browser/account you choose, and the resulting provider auth file is saved
only in that slice's persistent home volume. Use a different `ARROBA_SLICE_NAME` or
`ARROBA_SLICE_HOME_VOLUME` for another account-isolated slice.

For OpenCode Zen, use the OpenCode provider namespace and API-key login method:

```bash
ARROBA_SLICE_OPENCODE_PROVIDER=opencode \
ARROBA_SLICE_OPENCODE_LOGIN_METHOD="API Key" \
bash apps/kernel/slice-linux-docker/provision-linux-docker-slice.sh login-opencode
```

The matching logout helpers are:

```bash
bash apps/kernel/slice-linux-docker/provision-linux-docker-slice.sh logout-codex
bash apps/kernel/slice-linux-docker/provision-linux-docker-slice.sh logout-opencode
```

Start the slice kernel/relay and run provider-backed shell drills from inside the slice:

```bash
bash apps/kernel/slice-linux-docker/provision-linux-docker-slice.sh start-runtime
docker exec -u slice arroba-slice-linux \
  bash -lc 'ARROBA_KERNEL_PORT=43119 node /workspace/apps/shell/dist/shell.js run /workspace/experiments/slice-linux-docker/scripts/validate-kernel-session.arroba --workspace /workspace --worktree /workspace'
docker exec -u slice arroba-slice-linux \
  bash -lc 'ARROBA_KERNEL_PORT=43119 node /workspace/apps/shell/dist/shell.js run /workspace/experiments/slice-linux-docker/scripts/validate-kernel-session-opencode.arroba --workspace /workspace --worktree /workspace'
docker exec -u slice arroba-slice-linux \
  bash -lc 'ARROBA_KERNEL_PORT=43119 node /workspace/apps/shell/dist/shell.js run /workspace/experiments/slice-linux-docker/scripts/validate-kernel-session-opencode-zen.arroba --workspace /workspace --worktree /workspace'
```

Current Docker fallback limitations:

- Provider servers bind to container-local loopback for worker-kernel access; inspect them with
  `docker exec` rather than host `curl`.
- Debian's `chromium` package is used because Ubuntu's `chromium-browser` package is a Snap shim
  in containers.
- The container is created with Docker's seccomp profile disabled so Chromium can create its own
  sandbox namespaces. With Docker's default seccomp profile, Chromium aborts unless run with
  `--no-sandbox`.
- Credential files are kept in the `arroba-slice-linux-home` Docker volume. Importing host
  provider auth is explicit and copies provider-owned session files rather than ChatGPT passwords.
- X11 screen capture/viewing and browser control are separate in this spike. `xdotool` can move the
  virtual pointer, but Chromium input is currently driven through CDP because it was more reliable
  under Xvfb/Openbox in Docker.

## Slice V1 Shape

The spike is converging on this v1 shape:

1. Debian Docker image with Chromium, Codex CLI, OpenCode, Arroba kernel, and Arroba relay.
2. Xvfb/Openbox virtual desktop with optional noVNC user viewer.
3. Browser-control helper for Chromium.
4. Screen/OCR helper that returns screenshots and OCR text boxes.
5. Clipboard get/set helper.
6. Persistent provider/slice home volume.
7. User-selected worktree mount at `/workspace`.
8. Provider servers started inside the slice.
9. Later Arroba runtime MCP wrapper around the helpers.
10. Later pluggable backend/tool registry so users can swap OCR/display/browser helpers or add custom MCP tools.

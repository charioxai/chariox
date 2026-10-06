# MP-08 / MP-10 / MP-11 — sudo shell access

PR 10's launch and admission implementation is present in base
`74e50b787a5919ee5c3d580c5b088989fd4a1adf` (local 416 / relay 70).
PTY launches use portable-pty's Unix `setsid`; piped Claude launches call
`setsid` in the child before exec. Both preserve the normal provider, Seatbelt,
managed-isolation and environment-removal paths.

Unix peers are OS-identified. Sudo admission walks a rechecked ancestry chain
and selects its nearest kernel-tracked provider birth identity. Shared provider
roots fail closed. The live prompt must still name the sudo entry and provider
run. Every ancestor below that root must have been born after this turn's
admission: old helpers and their newly forked children cannot inherit a later
turn. Process-group and session membership never grant authority. Requests and
subscription deliveries recheck liveness; completion, interrupt and rotation
remove authority without relying on provider process exit. External grants keep
the separate D9 holder/descendant path; kernel-spawned agents cannot inherit them.

Focused reproduction:

```sh
cargo test -p chariox-kernel --lib sudo_shell -- --nocapture
cargo test -p chariox-kernel --lib kernel_access -- --nocapture
cargo test -p chariox-kernel --lib provider_launch -- --nocapture
cargo test -p chariox-kernel --lib managed_isolation -- --nocapture
```

On the shared builder, every Cargo invocation and Rust suite uses the reserved
compile lock, four build jobs, an explicit disposable external `CHARIOX_HOME`,
and this lane's target directory. To include the actual shell CLI, build
`@chariox/kernel-client` and `apps/shell`, then set
`CHARIOX_SUDO_SHELL_CLI` to the absolute `apps/shell/dist/shell.js` path and run
`sudo_shell_live_throwaway_kernel_cli_loses_authority_after_yield`.
The fixture launches a real PTY shell and real kernel websocket/Unix transport,
answers the product passkey interaction with its synthetic test passkey, runs
`session list` over `ws+unix`, completes the exact prompt through the kernel
completion seam, and repeats the command expecting refusal. The ignored child
fixture is explicitly executed by the parent; it is not a waived assertion.

The live fixture uses no provider account. It proves Linux launch/admission and
client behavior, not official-provider execution, macOS execution, signed-release
provenance, or fresh managed-versus-ordinary parity. MP-10 acceptance and the
MP-11 independent security-anchor review remain coordinator gates. No serialized
shape or protocol number changes in PR 10.

Evidence: `/root/.codex/evidence/browser-resume-20260930/kasudo/`.

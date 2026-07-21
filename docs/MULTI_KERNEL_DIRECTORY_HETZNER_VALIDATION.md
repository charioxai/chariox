# Multi-kernel directory Hetzner validation

Status: backend and provider matrix passed on 2026-07-21; product-browser screenshots pending.

## Topology

The drill used one local macOS machine and one Hetzner Linux machine. Each
machine ran two isolated feature kernels with distinct daemon IDs, aliases,
ports, state directories, and history directories. All four kernels shared one
feature relay. The Linux relay connection used an SSH reverse tunnel only for
the drill; hosted deployments continue to use the Caddy-fronted `wss://` relay.

The local waiting-room projection stabilized with:

- two online machines;
- one sibling kernel on the local machine (the current kernel is excluded from
  its own sibling count);
- two kernels on the Hetzner machine;
- schema version 10 with separate `structural_version` and
  `activity_revision` values.

A 45-second reconciliation watch sampled both local kernels every five seconds.
Every sample retained the same one-local-sibling/two-remote-kernel directory.

## Real provider matrix

Each row created a real session through the normal kernel protocol, attached a
client, launched the official provider harness, submitted a prompt, and verified
the exact reply from the kernel-owned durable history outline.

| Machine | Kernel | Provider | Result |
| --- | --- | --- | --- |
| local macOS | local A | Codex | `ARROBA_MATRIX_LOCAL_A_CODEX_OK` |
| local macOS | local B | OpenCode | `ARROBA_MATRIX_LOCAL_B_OPENCODE_OK` |
| Hetzner Linux | Hetzner A | Claude `-p` | `ARROBA_MATRIX_HETZNER_A_CLAUDE_OK` |
| Hetzner Linux | Hetzner B | Codex | `ARROBA_MATRIX_HETZNER_B_CODEX_OK` |

Claude Code correctly refused its bypass-permissions mode when the first Linux
kernel ran as root. The drill relaunched only that isolated kernel under the
machine's existing unprivileged `arroba-worker` account and then passed without
weakening the provider safety check. The kernel retained its state and history
directories across that relaunch.

## Reconnect and resource observations

When the drill's reverse tunnel expired, both Linux kernels remained healthy.
After the transport was restored, relaunching the two isolated kernels rebuilt
their relay registrations and both aliases became reachable again. The local
directory reconciled to the same two-machine/four-kernel topology.

Observed steady-state kernel RSS was approximately 68-94 MB per Linux kernel
and 76-80 MB per local kernel. The feature relay used about 9 MB RSS. The Linux
host retained about 2.9 GB of available memory after the provider turns.
OpenCode was the largest provider process observed at approximately 777 MB RSS.
After validation, every kernel-managed idle provider process was marked
`teardown_safe` and removed through `TeardownProviderProcesses`; the durable
sessions and history were retained for frontend validation.

## Remaining evidence

Use the product frontend in the in-app browser to capture the waiting room with
both machines and all four kernels visible, expand/select every kernel to
reconcile its saved session inventory, and confirm that the four matrix sessions
render without rerunning the providers. This evidence is intentionally deferred
until the browser webview is available; the local product server and matrix
topology remain running for that step.

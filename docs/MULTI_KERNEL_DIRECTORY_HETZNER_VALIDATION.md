# Multi-kernel directory Hetzner validation

Status: backend, real-provider, reconnect, resource, and product-frontend matrix
passed on 2026-07-21.

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
After each provider turn, kernel-managed idle processes that reported
`teardown_safe` were removed through `TeardownProviderProcesses`; the durable
sessions and history were retained for frontend validation. The Claude session
remained resumable at approximately 263 MB RSS and 0.2% CPU. During the final
Chrome check the Linux host still had approximately 3.0 GB available memory,
and opening the saved session did not launch another provider process.

## Product frontend evidence

The signed-in product frontend was validated in Chrome against the isolated
feature relay, kernels, and local-browser server. The waiting room displayed the
local macOS and Hetzner Linux machines, both kernels on each machine, and all
four durable sessions with their correct home kernel and provider. Each kernel
was selected so its live inventory reconciled, then the page was reloaded. All
four session aliases rendered immediately from saved inventory while the
selected kernel catalog reconciled lazily.

Opening `matrix-hetzner-a-claude-worker` from the waiting-room sidebar attached
the web terminal to the existing remote session and rendered both the original
prompt and exact `ARROBA_MATRIX_HETZNER_A_CLAUDE_OK` reply. This confirmed that
the optimistic inventory led to a real, attachable session rather than a stale
display-only row.

The drill server initially rejected the isolated local target because it was
configured with only a port and therefore resolved that port through the normal
user registry, whose older daemon identity did not match the isolated feature
kernel. The final drill configuration explicitly listed all four daemon IDs and
relay targets. That is test-environment identity isolation, not a product source
failure: hosted Cloud obtains the same target identities from the
heartbeat-backed repository.

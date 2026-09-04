# Real provider Room drill

Run the existing physical Room fixture with `CHARIOX_ROOM_DRILL_FOCUS=real-provider`,
`CHARIOX_ROOM_DRILL_PROVIDER=codex|claude|opencode`, and an explicit
`CHARIOX_ROOM_DRILL_MODEL`. Run one provider at a time. This makes a real provider
request and can consume provider usage.

Use `CARGO_TARGET_DIR` for the existing matching binaries and
`CHARIOX_ROOM_DRILL_IMAGE` for an existing exact-source image. The latter disables
automatic image builds. The fixture enforces one 2-GiB, one-CPU headed slice and
always cleans up its container, volume, temporary state, and listeners.

The real-provider mode explicitly enables the existing
`slices.linux.allow_unconfined_seccomp` option for this disposable local slice.
This permits the production Bubblewrap launcher to create its inner provider
namespace. The provider still runs with the inner seccomp filter, filesystem
isolation and dropped capabilities. Ordinary fixture modes and user settings
are unchanged. Do not use this local drill setting as managed-host acceptance;
managed hosts additionally require the dedicated rootless Docker boundary.

The kernel resolves the default linked provider profile and transfers it through
the normal slice-backed agent launch. The drill does not copy credentials or call
the provider's SDK, nor does it issue MCP calls on behalf of the agent. Local and
remote TUI observers still use stub agents; the separately spawned driver uses
the selected official provider.

Acceptance requires an agent-attributed completed Computer click in the Room
ledger, its physical click counter change in the shared browser, and matching
notices in both local and remote TUI. A textual provider success claim is not
acceptance. `real-provider.json` records the last completed phase; overall success
also requires `result.json` and successful cleanup.

`CHARIOX_ROOM_DRILL_IMPORT_FIRST=1` additionally runs the public slice account
import operation before spawning. Keep this separate from the normal automatic
transfer path so import-and-launch regressions can be reproduced.

This initial case does not yet validate structured Browser operations, Web
observation of that agent action, persistence, permission denial, or all three
providers. Those requirements remain in the end-to-end plan.

# Lane credcopies — FINAL

Branch: `cred/managed-login-copies`, based on GitHub main
`e325afa580d81954e2c179757fc53fa02ed2a2b3`. No push or GitHub CI.

Implemented the owner's copied-login model: track Chariox-performed transfers,
warn that copies can log the user out, offer official receiving-machine login,
and retain admitted/queued work behind a human-only recovery interaction.
There is no refresher broker or background credential synchronization.

Implementation follows audit order:

- G2: managed-context copy receipts/notices use the actual receiving profile,
  including default remaps. G3: OpenCode recognizes renewable OAuth entries
  across services. G4: managed Claude transfers accept only portable official
  login credentials; metadata-only/Vault setup-token launch stays separate.
- G1/G5/G7/G8: one copy recorder creates credential-free provenance. Context
  commits, lease/rebind/account selection, direct slice imports and publication
  bindings retain real source/target machine/kernel/account IDs. Confirmed
  receiving receipts preserve an existing receiver login and copy generation.
  Auth observations and removal tombstones update scoped copy records.
- TUI Provider Accounts list, selected-account details and command-center
  account descriptions show copies, warnings and receiving login commands.
- G6: bounded official auth errors share classification. Missing copied login
  artifacts also trigger recovery when an app-server has cached auth state.
  Existing interactions retain ownership; recovery waits for them, preserving
  the run and admitted prompt identity. Workspace-routing errors are excluded.

Local protocol **455**. Relay peer **74**; relay transport is unchanged.
The peer bump is required by direct slice import: the removed file-copy/Unix
helper path cannot obtain live kernel authority. The replacement is an encrypted
home-to-slice request checked against bootstrap slice ownership and the pinned
home public key. Old receivers must be upgraded before this operation.

## Live validation

Evidence: `/Users/miguel/.codex/evidence/credcopies/` (outside Git).
Real built Bun TUI, built kernel and relay, official installed Codex harness,
existing linked ChatGPT login, and a real kernel-managed leased worker ran here.
The drill never opened, printed or manually copied provider credential files.

**PASS:** Chariox copied the selected account; inventory contained actual machine
and kernel IDs; the built TUI showed the warning and receiving login command;
the worker answered `CREDCOPIES_LIVE_OK`; official `codex logout` removed the
receiving login and `codex login status` reported `Not logged in`.

**PASS after fix:** the actual receiving copy raised “Log in to Codex on this
machine”; the built TUI showed that request, with the admitted prompt retained
and a second prompt queued. This final recovery observation was made directly
on the original receiving kernel after restart, rather than claiming a new
successful home-worker reconnect. Source refresh returned workspace-routing 401
and restarting the ephemeral drill relay identity hit the existing changed-key
pin check. No trust pin was replaced to bypass that check.

**BLOCKED(owner):** new official receiving Codex login and queued-work resumption.
The human-only login question timed out without a user response. It was not
answered automatically; no new-login/resumption success is claimed.

**BLOCKED:** real OpenCode drill: `opencode auth list` reported zero credentials;
no alternative linked account was supplied. **BLOCKED(owner): Claude**:
`claude auth status --json` reported no linked login.

Managed-context/Path-1, direct Docker import, publication and home-worker
reconnect/rebind permutations still need real acceptance drills. Fixtures and
protocol tests below are supplementary, not replacements for these gates.

## Checks and cleanup

See `PUSH_READY.md` for final focused check results. Own live kernels, relay,
TUI and provider children were stopped. Durable account/identity state was
preserved; shared reviewer services and unrelated kernels were untouched.
Large compiler outputs are removed after retaining reviewable built binaries.

## Cloud follow-up (G9)

Pin the current `chariox-cloud` main and audit its Provider Accounts renderer.
Render shared `materializations[].copy`, the copied-login warning, observation
state/time and the actual receiving profile/machine. Use existing human-only
runtime login projection/input and require local protocol 455 for these fields.
Do not proxy terminal traffic or create a second credential policy.

Managed publication binding producers must supply `source: {machine_id,
kernel_id}` plus `source_account_id` for each staged managed account. The OSS
consumer fails loudly when managed provenance is absent; legacy same-machine,
unmanaged imports remain supported. Coordinate that producer change before
rollout. Cloud producer and renderer changes are not implemented in this lane.

# Provider account target scope — MP-08 / MP-10 / MP-11

Provider accounts belong to a kernel, within its existing account-owner namespace.
A signed-in account on one kernel is not evidence of a usable login on another.
Claude, claude-p and claude-headless share the Claude account family.

## Required real acceptance scenarios

These scenarios supplement the provider-account and ordinary/managed parity
matrix. Source tests do not close these rows.

| ID | Required behaviour |
| --- | --- |
| ACCT-TARGET-WEB | Pair two real kernels A and B to one Cloud user. Register a working Claude profile only on A. From a fresh desktop browser, select B through normal Machine/Kernel clicks. At DPR 1 (1920×1080 and 1440×900) and DPR 2, B must not offer A's login as signed in, including initial loading, switching, reconnect and reload. Record video/screenshots and review the recording for stuck or confusing states. Show RED on the frozen base and GREEN on the candidate. |
| ACCT-TARGET-TUI | Repeat A/B selection using the real built TUI. B's account choices and launch validation use B's inventory. An account on A is identified as living on A, with an explicit copy action rather than signed-in status on B. |
| ACCT-CLAUDE-FAMILY | A registered Claude profile resolves with claude, claude-p and claude-headless through normal kernel launch paths. A profile absent on B is still absent for every mode. Another user's profiles remain inaccessible. |
| ACCT-MISSING-DEFINITIVE | Launch with an absent profile, including removal after selection. The error identifies absence on this kernel, offers local sign-in or explicit copy from the source, and is nonretryable across local and relay transport. |
| ACCT-COPY-LOGIN | From B's waiting room, choose Copy login to this machine for A's account. Use existing kernel-owned consent, encrypted provider-login transfer, tracked operation/receipt and revocation boundaries. No credentials pass through Cloud or client projections. Denial/cancellation transfers nothing. The successful copy admits one short real claude-haiku-5-5 turn on B, with the resolved model recorded. Removing the copied login on B leaves A usable; no ongoing synchronization occurs. |

Use an owned hosted validation stack and two disposable kernel homes on b3.
Both homes must be explicit before startup; all process IDs, source identities,
binary hashes, commands, exit codes, resource samples and cleanup belong in
external evidence. Do not expose profile files, Vault data, tokens or device
credentials in screenshots, logs or evidence. Review security-critical account,
Vault and authenticated peer-admission anchors for MP-11.

The base's lease-bound EnsureRemoteProviderAccount operation is a transfer
primitive, not an ordinary-kernel UI copy operation: it requires an execution
lease and binds the recipient namespace to that lease's owner. Do not invent a
lease or bypass ownership/consent to satisfy the copy row. Serialized extensions
require coordinator-allocated protocol versions and matching snapshots/drills.

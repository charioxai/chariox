# One sudo turn (protocol 404, PR 8a)

In a Chariox terminal, focus a local regular agent and enter `/sudo <prompt>`.
The session host authorizes that entry in the kernel's passkey popup. Every
connected host terminal receives the same popup; guests cannot authorize it.
The critical-approval remember window does not satisfy sudo. Never type a
passkey into an agent transcript, provider prompt or shell command.

If the agent is busy, the authorized prompt waits in kernel memory for a fresh
turn. It never joins the durable prompt queue or steers the running turn.
`/kernel access list` shows pending and running sudo entries. Use
`/kernel access revoke <entry-id>` or `/kernel access revoke all` to revoke them.
Rotation revokes queued entries and interrupts running sudo turns. Restart
drops the authorization and records a notice; enter `/sudo` again to retry.
The submitting client waits for the popup and for an idle agent without a
response timeout or automatic replay. A lost connection fails the submission;
check the access list and revoke any remaining entry before retrying.

`chariox access list` now prints a JSON object with `grants` and `sudo_turns`
arrays. Scripts that previously consumed the grants array should select
`.grants`. The terminal `/kernel access list` includes both kinds of entry.

A sudo turn keeps ordinary provider tools and gains `chariox_kernel_request`
through the existing runtime MCP. Its `request` argument is one serialized
`LocalDaemonRequest`, for example:

```json
{"request":{"ListSessions":null}}
```

Critical replies use the shared interaction path, without supplying a passkey:

```json
{"request":{"RespondToInteraction":{"session_id":"target-session","interaction_id":"decision","choice_id":"approve"}}}
```

The kernel checks the live provider run, exact prompt and ephemeral sudo
binding on each call. Authority covers the host's kernel, including other
sessions and any number of critical approvals. It ends at yield or interruption;
there is no time or payment cap. Spawned and forked agents receive no sudo.
Sudo cannot mint access or sudo, answer credential-entry prompts, export secret
values, change the passkey or configure kernel access. Pairing, session invites,
relay configuration and Cloud identity operations remain host-terminal-only. Existing vault-entry
flows remain available through the ordinary runtime tools.

`kernel_access.sudo` records transitions. Each resolved sudo decision appends
`kernel_access.sudo_approval`, naming the authorizing entry, terminal, agent,
provider run, exact prompt, target interaction and choice. Command correlation
and causation also point to that turn and entry. Receipts contain no passkey.

This is the bounded PR 8a split of the kernel access plan. External Unix socket
sudo requests, the `/meta` migration notice and leased execution are follow-up
work. Existing Meta tasks keep their delegation-only behavior and must finish
before sudo entry. Shell CLI calls do not gain sudo; process-tree sudo is PR 10.
Cloud/native consumers must support the protocol-404 `sudo` popup kind before
advertising sudo entry. Owner passkey and real-client acceptance remain later
validation legs; the builder drill uses private test vaults and synthetic runs.

Run `scripts/kernel-access-sudo-drill.sh` on the Linux builder. It uses Rust
1.88.0 and the existing slot-run admission helper, covering queued revocation,
rotation, yield, interrupt, restart, critical receipts and protocol snapshots.

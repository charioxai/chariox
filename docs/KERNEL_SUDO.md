# One sudo turn (protocol 415)

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

MP-08 / MP-10 / MP-11: provider discovery advertises `chariox_kernel_request`
through the existing runtime MCP before the first turn, so official harnesses
can cache its interface. Ordinary turns cannot call it. A sudo turn activates
its authority while keeping ordinary provider tools. Its `request` argument
is one serialized
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

External agents holding a process-bound local-kernel grant can request a turn over
that kernel's Unix socket:

```sh
chariox sudo request --agent <agent-id> --prompt "<full prompt>" [--socket /absolute/kernel.sock]
```

The host's popup names the grant holder's OS executable and PID, the target
agent and session, and the full requester-supplied prompt. Only the host's
terminals can answer it. The external client receives the submission outcome;
it never receives or sends the passkey. TCP, relay and ungranted peers are refused. Any local session’s agent may be targeted; each request needs a fresh host passkey. One requester can have one pending
sudo request, which expires with a clear error if no terminal answers.
Grant expiry, process exit or revocation cancels a pending or queued external
request. The final dispatch boundary checks that the grant is still live.
Once the host-authorized turn starts, it follows the same one-turn lifetime as
terminal sudo. Rotation, revoke all and session end remove authorizations;
rotation and revoke all interrupt running turns. External request attribution
and the winning host terminal appear in sudo audit entries and receipts.

For one release `/meta` continues to run delegation-only tasks without a
passkey and displays a notice pointing to `/sudo`. Existing Meta tasks finish
in Meta mode; they must finish before sudo entry. Shell CLI calls use the tracked provider’s OS process tree for this sudo turn;
pre-existing descendant processes are excluded. Leased sudo execution remains a separate leg.
Cloud/native consumers must support the protocol-413 `sudo` popup kind before
advertising sudo entry. Owner passkey and real-client acceptance remain later
validation legs; the builder drill uses private test vaults and synthetic runs.

Run `scripts/kernel-access-sudo-drill.sh` on the Linux builder. It uses Rust
1.88.0 and the existing slot-run admission helper, covering queued revocation,
rotation, session end, yield, interrupt, restart, critical receipts, external
Unix requests, the Meta notice and protocol snapshots.

MP-08 / MP-10 / MP-11: the shared router also filters every response to a
local grant or sudo MCP caller. Credential read and mutation replies redact
literal injection values. Raw MCP/connector configuration, native login
output/codes, pairing/Cloud admission credentials and enrollment callbacks
are withheld; inspect those through the host terminal. An operation can complete
while its reply is withheld; check host-terminal state before retrying. Executor failures use
a value-free error because parser/provider diagnostics can echo secrets. Exact
constant refusal, expiry and revocation diagnostics remain available.
This preserves normal workspace/history authority and is not a file sandbox.

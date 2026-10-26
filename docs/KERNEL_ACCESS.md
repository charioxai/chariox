# Kernel and terminal access

One Cloud account is one user. My kernels lists that account's independently enrolled kernels grouped by machine. Invited collaborators enter shared sessions; their kernels never appear in your directory.

Use `/cloud link` on each kernel to enroll it. The kernel keeps its own durable credential in private 0600 state. Attached terminals can pivot through this enrolled kernel without a terminal login. Short, exact-target, key-bound grants renew silently from the issuing kernel's durable credential. Unlinking that issuer retires its delegated grants; sibling kernels remain independent.

`chariox cloud login` and detached waiting-room Login enroll only the terminal profile, without starting a kernel. The profile keeps one stable client identity and a rotating refresh family in a private 0600 store. Active access and relay grants renew without reconnection or fixed-lifetime prompts. Revocation or detected refresh reuse stops the terminal. `chariox cloud logout` revokes that terminal only. Use separate absolute CHARIOX_HOME roots for separate accounts; conflicting accounts or Cloud origins fail clearly. Public preferences/status contain no credentials. Browser My kernels → Signed-in terminals offers Revoke.

## Self-hosted operator realm

Cloud is optional. Configure an operator-controlled relay and each kernel's relay connection. Live relay registration/heartbeats provide discovery. Open the terminal-pairing dialog in the attached kernel's waiting room, then use the issued link with `chariox --terminal-pairing-link <link>`. The link is private; do not log it or commit it. The terminal keeps its existing private profile key. Pairing records a durable key-bound grant at that exact kernel; operator transport access alone cannot read runtime inventory or subscribe. Revoke the paired terminal at the kernel to stop requests and existing event streams. Grants have no fixed lifetime; restarting with the same CLI profile key preserves admission.

The simplest realm uses a shared transport token. Operators rotate that shared token to revoke relay-level realm access; per-terminal relay credential revocation is not promised for a shared token. Kernel pairing revocation remains independent.

For a scoped HMAC realm, provision the kernel's KERNEL transport token and a separate CLIENT transport token for terminals. The issuer secret stays with the operator/relay; it is not required in the kernel or CLI. Pairing links never export a scoped kernel transport credential; generic legacy invites reject exporting it as well. Supply the operator-issued CLIENT token through an environment variable with `chariox --terminal-pairing-link <link> --relay-token-env <variable-name>` (either flag order works). Its transport actions are client.metadata.read, client.connect and packet.route in the operator realm. Runtime/session authority still comes from the kernel key grant. The operator controls transport credential issuance and rotation; scoped issuer tooling and a self-hosted login/directory service are outside this baseline.

Protocol 478 adds the kernel pairing confirmation. Relay peer protocol is unchanged. The reusable local drill is `node apps/cli/scripts/live-self-host-terminal-pairing-drill.mjs`; add `--scoped` for HMAC. Build the kernel, relay and CLI first. The drill creates and removes explicit disposable profiles, uses private ports, never starts a provider run, and records only statuses. It verifies zero Cloud requests, key-bound join, restart reuse, foreign-key rejection, private file mode and live revocation.

## For agents

An external agent (Claude Code, Codex, OpenCode) talks only to its local kernel over the Unix socket. Run `chariox guide` for the short, static agent guide; it is generated from the same command table the CLI parser uses and installs nothing into the agent's harness.

- `chariox access request --until-exit` asks the user for access once. With `--no-wait` it exits 6 and a detached waiter keeps the request; `chariox access status` then prints `granted`, `pending`, `ended` or `not_requested`.
- `chariox kernels` lists the kernels this kernel can reach, grouped by machine. A kernel's name is its machine's name when you have one kernel there and `<machine>/<label>` when the machine runs several (a name that fits two kernels is `kernel_ambiguous` and lists both). `chariox request [--kernel M] [--command-id ID] [--timeout SECONDS]` sends one `LocalDaemonRequest` read from stdin to this kernel, or through it to kernel `M`, and prints the response JSON.
- `chariox watch --kernel M --session ID --attachment ID [--from EVENT_ID]` prints one JSON line per event of that kernel until the connection is lost; it then ends with `kernel_connection_lost` naming the event to resume from. An attachment made through the gateway is kept alive by this kernel while the grant lasts, and ends with `DetachFromSession`.
- `chariox files get|put --kernel M ...` moves files in chunks of 256 KiB, resumable (2 GiB per file); run the same command again after an interruption. `put` is verified by sha256 on the target; `get` prints the sha256 of what it wrote and `--sha256 HEX` fails on a mismatch. Listing, stat, hashing and small reads go through `chariox terminal` (`ls -la`, `stat`, `sha256sum`, `head -c`).
- A person pairs this kernel with a self-hosted kernel with `/kernel add <link>` in a Chariox terminal (`/kernel linked`, `/kernel unlink NAME`); agents cannot.
- Output is one JSON document on stdout. Errors are `{"error":{"code","message","kernel"}}` on stderr.

Exit codes and the error codes behind them:

| Exit | Meaning | Codes |
| --- | --- | --- |
| 0 | ok | |
| 2 | usage | `usage` |
| 3 | refused | `kernel_sudo_refused`, `kernel_sudo_room_owner_local`, `kernel_access_human_only`, `kernel_access_ended`, `kernel_access_not_covered`, `kernel_access_already_granted`, `command_id_required`, `duplicate_command_conflict` |
| 4 | kernel unavailable | `kernel_unknown`, `kernel_ambiguous`, `kernel_offline`, `kernel_unreachable`, `kernel_unpaired`, `kernel_incompatible`, `kernel_not_available` |
| 5 | timeout or unknown outcome | `kernel_sudo_expired`, `kernel_request_timeout`, `kernel_connection_lost`, `kernel_outcome_unknown` |
| 6 | approval pending | |

On `kernel_request_timeout` or `kernel_connection_lost`, retry once with the same `--command-id`. On `kernel_outcome_unknown`, read the state first. When the local connection drops with a request in flight, the CLI probes the kernel once: `kernel_access_ended` means the grant ended during the request; anything else is `kernel_connection_lost`.


MP-08 / MP-10 / MP-11: `kernel_sudo_refused` means the person refused the request;
`kernel_sudo_expired` means no approval arrived before its deadline. Neither
starts elevation or retries automatically. Submit a new request only when needed,
and approve it in an already paired Chariox client. The launcher and the bare
TypeScript CLI both walk past shells to the nearest installed provider harness;
no `CHARIOX_CLI_ACCESS_HOLDER_PID` override is needed for ordinary use.

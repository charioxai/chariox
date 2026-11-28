# Kernel and terminal access

One Cloud account is one user. My kernels lists that account's independently enrolled kernels grouped by machine. Invited collaborators enter shared sessions; their kernels never appear in your directory.

Use `/cloud link` on each kernel to enroll it. The kernel keeps its own durable credential in private 0600 state. Attached terminals can pivot through this enrolled kernel without a terminal login. Short, exact-target, key-bound grants renew silently from the issuing kernel's durable credential. Unlinking that issuer retires its delegated grants; sibling kernels remain independent.

`chariox cloud login` and detached Room list Login enroll only the terminal profile, without starting a kernel. The profile keeps one stable client identity and a rotating refresh family in a private 0600 store. Active access and relay grants renew without reconnection or fixed-lifetime prompts. Revocation or detected refresh reuse stops the terminal. `chariox cloud logout` revokes that terminal only. Use separate absolute CHARIOX_HOME roots for separate accounts; conflicting accounts or Cloud origins fail clearly. Public preferences/status contain no credentials. Browser My kernels → Signed-in terminals offers Revoke.

## Self-hosted operator realm

Run the kernel as a normal user. If you run it as root, Chariox passes
`IS_SANDBOX=1` to YOLO Claude agents using an undocumented Claude variable.
See the [root YOLO compatibility drill](RUNNING_LOCAL.md#claude-yolo-when-self-hosting-as-root)
for the check to run after each Claude version bump.

Cloud is optional. Configure an operator-controlled relay and each kernel's relay connection. Live relay registration/heartbeats provide discovery. Open the terminal-pairing dialog in the attached kernel's Room list, then use the issued link with `chariox --terminal-pairing-link <link>`. The link is private; do not log it or commit it. The terminal keeps its existing private profile key. Pairing records a durable key-bound grant at that exact kernel; operator transport access alone cannot read runtime inventory or subscribe. Revoke the paired terminal at the kernel to stop requests and existing event streams. Grants have no fixed lifetime; restarting with the same CLI profile key preserves admission.

The simplest realm uses a shared transport token. Operators rotate that shared token to revoke relay-level realm access; per-terminal relay credential revocation is not promised for a shared token. Kernel pairing revocation remains independent.

For a scoped HMAC realm, provision the kernel's KERNEL transport token and a separate CLIENT transport token for terminals. The issuer secret stays with the operator/relay; it is not required in the kernel or CLI. Pairing links never export a scoped kernel transport credential; generic legacy invites reject exporting it as well. Supply the operator-issued CLIENT token through an environment variable with `chariox --terminal-pairing-link <link> --relay-token-env <variable-name>` (either flag order works). Its transport actions are client.metadata.read, client.connect and packet.route in the operator realm. Runtime/session authority still comes from the kernel key grant. The operator controls transport credential issuance and rotation; scoped issuer tooling and a self-hosted login/directory service are outside this baseline.

Protocol 478 adds the kernel pairing confirmation. Relay peer protocol is unchanged. The reusable local drill is `node apps/cli/scripts/live-self-host-terminal-pairing-drill.mjs`; add `--scoped` for HMAC. Build the kernel, relay and CLI first. The drill creates and removes explicit disposable profiles, uses private ports, never starts a provider run, and records only statuses. It verifies zero Cloud requests, key-bound join, restart reuse, foreign-key rejection, private file mode and live revocation.

## For agents

An external agent (Claude Code, Codex, OpenCode) talks only to its local kernel over the Unix socket. It needs no environment variables: with no `--socket`, `CHARIOX_DAEMON_SOCKET` or `CHARIOX_HOME`, the CLI uses the one local kernel with a fresh heartbeat, in the default home or in a Setup home (`~/.chariox/dev/machines/<id>`). With none running it exits 4 (`kernel_unreachable`) and says what to start; with several it exits 4 (`kernel_ambiguous`) and lists each one's home, port and exact `--socket` (or `CHARIOX_HOME`) to choose it. Only heartbeats count, never a retained kernel identity, and a selected `CHARIOX_KERNEL_PORT` is never answered by another kernel. Setup's last lines print the CLI's full path (`~/.local/bin/chariox`) and the PATH step. Run `chariox guide` for the short, static agent guide; it is generated from the same command table the CLI parser uses and installs nothing into the agent's harness.

- `chariox access request --until-exit` asks the user for access once. The kernel picks the holder from the caller's host process ancestry: the installed Codex, Claude Code or OpenCode that runs the shell, skipping a provider sandbox's per-command PID namespace and helper processes, so it works inside Codex's standard `workspace-write` sandbox when the kernel socket's directory is writable. `--holder-pid` names a host pid of the caller or an OS ancestor instead. In an agent shell with a command timeout use `--no-wait`: the kernel records the request, raises the popup and the command exits 6 at once; the decision binds to the holder, so the shell (and its sandbox) may end before the owner answers. Poll `chariox access status` until it prints `granted` (`pending` while the user decides; `ended` with a `reason` when the user refused, the request timed out or was invalidated, or the grant ended; `not_requested` otherwise). Status is about that one holder: a pid that is not the caller or its ancestor is refused, never answered with the caller's grant. The grant lasts until the holder exits and survives a kernel restart while the holder lives: after a restart run `chariox access status` before asking again (a timed grant ends with the restart).
- `chariox access cover --pid PID` keeps your access for a background process you started (`nohup tool &`, then `chariox access cover --pid $!`) after its parent exits, until that process exits or the grant ends. Run it right after starting the process, or from the process itself; `chariox access status --holder-pid $$` then prints `granted` for it. A grant covers at most 16 processes this way. Programs already running under the approved agent when you approve (a dispatcher it started before it asked) are covered by identity at that moment, up to 48, so they keep access after the shell that started them exits; a program whose parent had already exited before the approval is no longer under the agent and is not covered. A process that forks twice gets a new pid and can't be covered.
- `chariox secret request CREDENTIAL_ID --label TEXT` asks the user for a value that goes straight into their Vault. It prints the link as its first line (show it to the user verbatim, do not open it, never ask for the value in chat), then waits and prints `{interaction_id, credential_id, status}` with `stored` (exit 0), `cancelled` (exit 3) or `timed_out` (exit 5). `--no-wait` exits 6 after the link; `chariox secret status INTERACTION_ID` reads the outcome later. The user types the value in a connected Chariox client or on the link's page, behind their Cloud sign-in; the agent never receives it and uses the stored value through its credential handle (default handle: `Authorization: Bearer ${secret}` over HTTP). `--overwrite` replaces an existing handle's value. Without a page for the link (`secret_link.page_url`, defaulted from the enrolled Cloud) the request fails with `secret_link_unavailable`: ask the user to answer in a connected Chariox client.
- `chariox kernels` lists the kernels this kernel can reach, grouped by machine. A kernel's name is its machine's name when you have one kernel there and `<machine>/<label>` when the machine runs several (a name that fits two kernels is `kernel_ambiguous` and lists both). `chariox request [--kernel M] [--command-id ID] [--timeout SECONDS]` sends one `LocalDaemonRequest` read from stdin to this kernel, or through it to kernel `M`, and prints the response JSON. `--timeout` is how long the CLI waits for the answer (default 30 seconds). A forwarded request also allows for a long read or terminal wait.
- `chariox exec [--kernel M] [--directory PATH | --workspace PATH | --session ROOM [--host-agent AGENT]] [--timeout SECONDS] -- argv...` streams binary stdin and separate stdout/stderr. Kernel and workspace targets run directly on that kernel without creating a Room; a Room target uses its workspace or named agent slice. No supplied input sends EOF. For a kernel command, `--stdin` waits for local input and `--stdin-file KERNEL_PATH` streams a regular file already on the target; inline input remains bounded to 64 KiB. The kernel owns the deadline and returns its observed exit state. Known output cursors recover a brief disconnect within the original remaining deadline and reader lease; an uncertain open is never replayed.
- `chariox watch --kernel M --session ID --attachment ID [--from EVENT_ID]` prints one JSON line per event of that kernel until the connection is lost; it then ends with `kernel_connection_lost` naming the event to resume from. An attachment made through the gateway is kept alive by this kernel while the grant lasts, and ends with `DetachFromSession`.
- `chariox files stat|ls|get|put --kernel M ...` moves files in chunks of 256 KiB, resumable and verified by sha256 (2 GiB per file); run the same command again after an interruption.
- A person pairs this kernel with a self-hosted kernel with `/kernel add <link>` in a Chariox terminal (`/kernel linked`, `/kernel unlink NAME`); agents cannot.
- Kernel requests print one JSON document on stdout; command streams are binary. Errors are `{"error":{"code","message","kernel"}}` on stderr.

### The owner's agents act as the owner (row 405)

An agent of the kernel's owner has the owner's authority on every kernel of the owner, with no card, passkey or `/sudo`: exactly what the owner's own client can do. Only payments and secrets go through the person: billing, passkey critical approvals, opening or unlocking the Vault, setting secrets, raw credential and MCP registry reads, provider logins, pairing and invite links, relay and Cloud identity tokens, and the unredacted debug bundle. Settings read without their secret-bearing sections. Chariox-managed rentals, paid ones included, follow only `agents.offload_without_asking`: on means no card, off means exactly one card; only the owner sets that toggle.

- The tool: `chariox_kernel_request` takes one `LocalDaemonRequest` and, with `kernel`, runs it on another kernel of the owner through this kernel's gateway (`ListGatewayKernels` lists them). An action card (`request_popup` with `action`) is the agent asking the person: the card is always shown and nothing runs until they approve (Cancel runs nothing); only a read of the agent's own Room runs at once.
- The shell: the `chariox` CLI run from the agent's own process tree acts as the agent, here or with `--kernel` on another kernel of the owner. No `chariox access request` is needed; it answers that access is already granted.
- Identity: the agent's own live provider run (`reach:<run>`), never a token. The target kernel admits the request because it arrives from a gateway kernel paired to its owner over the relay, and it applies the same refusal list itself.
- Agents of another account, and guests' agents, are refused: "only agents of this kernel's owner reach other rooms and kernels".
- An agent leased to a worker kernel acts the same way when the worker and home belong to the same Cloud user and realm: the worker forwards its `chariox_kernel_request` home, which judges it as that agent. Other workers, including a home without Cloud owner data, need a live home-prompt sudo window and cannot target another kernel.
### Room agent access

An agent of the kernel's owner needs none of this (see above). Any other Room agent, including a sub-agent, can ask the kernel owner for access. The kernel identifies the
agent from its running provider process. The provider shell gets the matching installed CLI as
`CHARIOX_CLI` and the Unix socket address as `CHARIOX_DAEMON_SOCKET`. In a source installation,
start the kernel with `CHARIOX_CLI_BIN` set to the absolute path of the CLI built from the same revision
if the CLI is installed separately.

```sh
"$CHARIOX_CLI" access request --no-wait --reason "Check the target kernel OS"
"$CHARIOX_CLI" access status
"$CHARIOX_CLI" exec --kernel <other-kernel> -- uname -a
```

The owner sees one card with the requester, Room and reason. They can grant timed access, grant
access until the agent's provider exits, or deny the request. Only run the command after status
reports `granted`. While pending, status gives the time the request will expire. A denial and an
unanswered expiry have distinct outcomes. Standing external grants do not apply to Room agents.

### Development standing grants

By default a grant lasts only while the holder's process does: when the agent's process restarts, it asks the owner again. On a development kernel the owner can pre-approve a holder so that restart needs no popup. Edit the user config file by hand (there is no command, popup or API for it, so no agent can enable it for itself):

```toml
[[kernel_access.standing_holders]]
harness = "claude"                        # codex, claude or opencode
executable = "/usr/local/bin/claude"      # the holder's exact executable path, as the kernel shows it in the access popup
```

A request that asks for `--until-exit`, from a holder with that harness and that executable, is admitted at once, without a popup. It is an ordinary until-exit grant: it ends when the holder exits, comes back after a kernel restart while the holder lives, shows in `chariox access list`, and `chariox access revoke` ends it. The audit record says `granted` with the reason `standing_policy`. Revoking does not stop the holder's next request from being admitted again: remove the entry to stop that. The file is read at each request, so adding or removing an entry needs no restart; a malformed entry is ignored with a log line, and a timed (`--minutes`) request always asks the owner.

Trade-off: any process of that executable and harness under the same user can obtain access, so keep the list empty on shared or production kernels. It is empty by default.

Exit codes and the error codes behind them:

| Exit | Meaning | Codes |
| --- | --- | --- |
| 0 | ok | |
| 1 | request failed: the kernel answered with an error code of its own; read `code` and `message` | any code not listed below, for example `session_not_found`, `file_not_found`, `ledger_unavailable` |
| 2 | usage | `usage`, `file_path_invalid` |
| 3 | refused | `exec_access_denied`, `events_refused`, `acquisition_denied`, `approval_required`, `secret_link_unavailable`, `kernel_sudo_refused`, `kernel_sudo_room_owner_local`, `kernel_access_human_only`, `kernel_access_ended`, `kernel_access_not_covered`, `kernel_access_already_granted`, `kernel_access_cover_gone`, `kernel_access_cover_identity_changed`, `kernel_access_cover_kernel_process`, `kernel_access_cover_outside`, `kernel_access_cover_limit`, `kernel_access_agent_unavailable`, `kernel_access_local_only`, `kernel_access_request_denied`, `command_id_required`, `duplicate_command_conflict`, `file_permission_denied`, `subscription_ended` |
| 4 | kernel unavailable | `kernel_unknown`, `kernel_ambiguous`, `kernel_offline`, `kernel_unreachable`, `kernel_unpaired`, `kernel_incompatible`, `kernel_update_needed`, `kernel_not_available` |
| 5 | timeout or unknown outcome | `exec_timed_out`, `kernel_sudo_expired`, `kernel_access_request_expired`, `kernel_request_timeout`, `kernel_connection_lost`, `kernel_outcome_unknown`, `kernel_access_unconfirmed`, `steer_unconfirmed` |
| 6 | approval pending (`access request --no-wait`, `secret request --no-wait`, `secret status` while pending) | |
| 7 | event cursor expired: resync followed targets | `cursor_expired` |

Forwarded Room writes return their changed rows and the Room ID in `session_id` (`session_ids` for multiple Rooms). A prompt receipt names the prompt by ID. Fetch current Room state with `GetSessionState` and prompt content with the history or queue reads.

If the reply details exceed the durable receipt limit (about 256 KiB), exit 0 with `GatewayWriteCompleted` confirms that the kernel completed the request. Its `response_kind` identifies the result whose details were omitted. A shell command includes its own `exit_code`. Read the current state to see the effect. For commands that produce large output, write the output to a file on that kernel and read it with `chariox files get`.

On `kernel_request_timeout`, `kernel_connection_lost` or `kernel_access_unconfirmed` (the kernel could not read the caller's process just now; the grant has not changed), retry once with the same `--command-id`. A steer that returns `steer_unconfirmed` may still arrive. Retry once with the command ID in the message. To learn whether a timed-out command ran before retrying, send `GetGatewayCommandStatus { command_id }` through the same gateway and grant (`chariox request --kernel NAME`): it answers `not_run`, `running`, `ran` with the original result, `interrupted` (the kernel restarted before the receipt; the effect is unknown) or `expired`. On `kernel_outcome_unknown`, read the state first. When the local connection drops with a request in flight, the CLI probes the kernel once: `kernel_access_ended` means the grant ended during the request; anything else is `kernel_connection_lost`.


MP-08 / MP-10 / MP-11: `kernel_sudo_refused` means the person refused the request;
`kernel_sudo_expired` means no approval arrived before its deadline. Neither
starts elevation or retries automatically. Submit a new request only when needed,
and approve it in an already paired Chariox client. The kernel, not the CLI,
picks the access holder from the Unix peer's host ancestry, so no holder pid is
needed for ordinary use, including from a sandboxed tool shell.

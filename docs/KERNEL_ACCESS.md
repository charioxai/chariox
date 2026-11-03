# Kernel and terminal access

One Cloud account is one user. My kernels lists that account's independently enrolled kernels grouped by machine. Invited collaborators enter shared sessions; their kernels never appear in your directory.

Use `/cloud link` on each kernel to enroll it. The kernel keeps its own durable credential in private 0600 state. Attached terminals can pivot through this enrolled kernel without a terminal login. Short, exact-target, key-bound grants renew silently from the issuing kernel's durable credential. Unlinking that issuer retires its delegated grants; sibling kernels remain independent.

`chariox cloud login` and detached waiting-room Login enroll only the terminal profile, without starting a kernel. The profile keeps one stable client identity and a rotating refresh family in a private 0600 store. Active access and relay grants renew without reconnection or fixed-lifetime prompts. Revocation or detected refresh reuse stops the terminal. `chariox cloud logout` revokes that terminal only. Use separate absolute CHARIOX_HOME roots for separate accounts; conflicting accounts or Cloud origins fail clearly. Public preferences/status contain no credentials. Browser My kernels → Signed-in terminals offers Revoke.

## Self-hosted operator realm

Run the kernel as a normal user. If you run it as root, Chariox passes
`IS_SANDBOX=1` to YOLO Claude agents using an undocumented Claude variable.
See the [root YOLO compatibility drill](RUNNING_LOCAL.md#claude-yolo-when-self-hosting-as-root)
for the check to run after each Claude version bump.

Cloud is optional. Configure an operator-controlled relay and each kernel's relay connection. Live relay registration/heartbeats provide discovery. Open the terminal-pairing dialog in the attached kernel's waiting room, then use the issued link with `chariox --terminal-pairing-link <link>`. The link is private; do not log it or commit it. The terminal keeps its existing private profile key. Pairing records a durable key-bound grant at that exact kernel; operator transport access alone cannot read runtime inventory or subscribe. Revoke the paired terminal at the kernel to stop requests and existing event streams. Grants have no fixed lifetime; restarting with the same CLI profile key preserves admission.

The simplest realm uses a shared transport token. Operators rotate that shared token to revoke relay-level realm access; per-terminal relay credential revocation is not promised for a shared token. Kernel pairing revocation remains independent.

For a scoped HMAC realm, provision the kernel's KERNEL transport token and a separate CLIENT transport token for terminals. The issuer secret stays with the operator/relay; it is not required in the kernel or CLI. Pairing links never export a scoped kernel transport credential; generic legacy invites reject exporting it as well. Supply the operator-issued CLIENT token through an environment variable with `chariox --terminal-pairing-link <link> --relay-token-env <variable-name>` (either flag order works). Its transport actions are client.metadata.read, client.connect and packet.route in the operator realm. Runtime/session authority still comes from the kernel key grant. The operator controls transport credential issuance and rotation; scoped issuer tooling and a self-hosted login/directory service are outside this baseline.

Protocol 478 adds the kernel pairing confirmation. Relay peer protocol is unchanged. The reusable local drill is `node apps/cli/scripts/live-self-host-terminal-pairing-drill.mjs`; add `--scoped` for HMAC. Build the kernel, relay and CLI first. The drill creates and removes explicit disposable profiles, uses private ports, never starts a provider run, and records only statuses. It verifies zero Cloud requests, key-bound join, restart reuse, foreign-key rejection, private file mode and live revocation.

## For agents

An external agent (Claude Code, Codex, OpenCode) talks only to its local kernel over the Unix socket. Run `chariox guide` for the short, static agent guide; it is generated from the same command table the CLI parser uses and installs nothing into the agent's harness.

- `chariox access request --until-exit` asks the user for access once. The kernel picks the holder from the caller's host process ancestry: the installed Codex, Claude Code or OpenCode that runs the shell, skipping a provider sandbox's per-command PID namespace and helper processes, so it works inside Codex's standard `workspace-write` sandbox when the kernel socket's directory is writable. `--holder-pid` names a host pid of the caller or an OS ancestor instead. In an agent shell with a command timeout use `--no-wait`: the kernel records the request, raises the popup and the command exits 6 at once; the decision binds to the holder, so the shell (and its sandbox) may end before the owner answers. Poll `chariox access status` until it prints `granted` (`pending` while the user decides; `ended` with a `reason` when the user refused, the request timed out or was invalidated, or the grant ended; `not_requested` otherwise). Status is about that one holder: a pid that is not the caller or its ancestor is refused, never answered with the caller's grant. The grant lasts until the holder exits and survives a kernel restart while the holder lives: after a restart run `chariox access status` before asking again (a timed grant ends with the restart).
- `chariox access cover --pid PID` keeps your access for a background process you started (`nohup tool &`, then `chariox access cover --pid $!`) after its parent exits, until that process exits or the grant ends. Run it right after starting the process, or from the process itself; `chariox access status --holder-pid $$` then prints `granted` for it. A grant covers at most 16 processes this way. A process that forks twice gets a new pid and can't be covered.
- `chariox secret request CREDENTIAL_ID --label TEXT` asks the user for a value that goes straight into their Vault. It prints the link as its first line (show it to the user verbatim, do not open it, never ask for the value in chat), then waits and prints `{interaction_id, credential_id, status}` with `stored` (exit 0), `cancelled` (exit 3) or `timed_out` (exit 5). `--no-wait` exits 6 after the link; `chariox secret status INTERACTION_ID` reads the outcome later. The user types the value in a connected Chariox client or on the link's page, behind their Cloud sign-in; the agent never receives it and uses the stored value through its credential handle (default handle: `Authorization: Bearer ${secret}` over HTTP). `--overwrite` replaces an existing handle's value. Without a page for the link (`secret_link.page_url`, defaulted from the enrolled Cloud) the request fails with `secret_link_unavailable`: ask the user to answer in a connected Chariox client.
- `chariox kernels` lists the kernels this kernel can reach, grouped by machine. A kernel's name is its machine's name when you have one kernel there and `<machine>/<label>` when the machine runs several (a name that fits two kernels is `kernel_ambiguous` and lists both). `chariox request [--kernel M] [--command-id ID] [--timeout SECONDS]` sends one `LocalDaemonRequest` read from stdin to this kernel, or through it to kernel `M`, and prints the response JSON. `--timeout` is how long the CLI waits for the answer (default 30 seconds); a forwarded request is also waited for by this kernel for 30 seconds, plus the request's own wait: a `RunShellCommand`'s `timeout_ms` (up to one hour) or a terminal wait. So `RunShellCommand` with `"timeout_ms": 400000` needs `--timeout 430` or more.
- `chariox watch --kernel M --session ID --attachment ID [--from EVENT_ID]` prints one JSON line per event of that kernel until the connection is lost; it then ends with `kernel_connection_lost` naming the event to resume from. An attachment made through the gateway is kept alive by this kernel while the grant lasts, and ends with `DetachFromSession`.
- `chariox files stat|ls|get|put --kernel M ...` moves files in chunks of 256 KiB, resumable and verified by sha256 (2 GiB per file); run the same command again after an interruption.
- A person pairs this kernel with a self-hosted kernel with `/kernel add <link>` in a Chariox terminal (`/kernel linked`, `/kernel unlink NAME`); agents cannot.
- Output is one JSON document on stdout. Errors are `{"error":{"code","message","kernel"}}` on stderr.

### Agent reach (Room agents)

A Room agent of the kernel owner reaches other rooms and the user's other kernels through its runtime MCP, which carries a second server, `chariox-reach` (`/mcp/reach`), next to `chariox`. It has two tools: `kernels` (the kernels the user can reach) and `request` (one `LocalDaemonRequest`, optionally with a `kernel`). There is no CLI or socket door for Room agents, and no off switch for the owner's agents. An agent of a guest never gets the tools.

The identity is the agent's own live provider run (`reach:<run>`); it ends with the run. The kernel authorises it as the owner, with the same rule as at home:

| The request | Without /sudo | In a live /sudo window of the agent's own turn |
| --- | --- | --- |
| Discover and read: sessions, agents, history, recall | allowed | allowed |
| Attach, message and steer agents; a room's terminals and workspace files; machines (`ControlPlane`, except deleting a saved state) | allowed | allowed |
| Anything else a /sudo window adds at home (settings, extensions, renames, kernel-wide files, ...) | refused: `reach_requires_sudo` | allowed |
| The /sudo exclusion list (`RequestKernelSudo`, `RespondToInteraction`, deletes, events, ...) | refused: `excluded_for_agents` | refused |
| The human-only set (passkeys, Vault, pairing, relay, Cloud identity, protected settings) | refused: `human_only` | refused |

A request for another kernel goes through the owner's gateway (the same one `chariox request --kernel` used); `on_behalf_of.harness` names `<room>/<agent> (Chariox agent)`, so the target's audit rows and its paired-clients list show which agent acted. Authority is enforced on the agent's own kernel.

Acquiring a machine (`ControlPlane` acquire, start and start-from-saved-state) needs no question by default. With the setting `agents.ask_before_acquiring_machines = true` ("Ask each time an agent acquires a machine through your connected service"; off by default; owner-only) each such request raises one decision in the agent's room, shown on every client; declining answers `acquisition_declined`.

Drill: `node scripts/drills/agent-reach.mjs` (two real kernels, local relay, a real Claude agent).

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
| 1 | request failed: the kernel answered with an error code of its own; read `code` and `message` | any code not listed below, for example `session_not_found`, `file_not_found`, `ledger_unavailable`, `kernel_response_too_large` |
| 2 | usage | `usage`, `file_path_invalid` |
| 3 | refused | `events_refused`, `secret_link_unavailable`, `kernel_sudo_refused`, `kernel_sudo_room_owner_local`, `kernel_access_human_only`, `kernel_access_ended`, `kernel_access_not_covered`, `kernel_access_already_granted`, `kernel_access_cover_gone`, `kernel_access_cover_identity_changed`, `kernel_access_cover_kernel_process`, `kernel_access_cover_outside`, `kernel_access_cover_limit`, `command_id_required`, `duplicate_command_conflict`, `file_permission_denied`, `subscription_ended` |
| 4 | kernel unavailable | `kernel_unknown`, `kernel_ambiguous`, `kernel_offline`, `kernel_unreachable`, `kernel_unpaired`, `kernel_incompatible`, `kernel_not_available` |
| 5 | timeout or unknown outcome | `kernel_sudo_expired`, `kernel_request_timeout`, `kernel_connection_lost`, `kernel_outcome_unknown`, `kernel_access_unconfirmed` |
| 6 | approval pending (`access request --no-wait`, `secret request --no-wait`, `secret status` while pending) | |
| 7 | event cursor expired: resync followed targets | `cursor_expired` |

`kernel_response_too_large` means a state-changing request through the gateway (a `RunShellCommand` that prints a lot, for example) ran and finished, but its answer is more than the gateway can keep as a receipt (about 256 KiB; the message names the exact limit). It is not run again. Write the output to a file on that kernel and read it with `chariox files get`, or print less. Answers of reads are not limited this way.

On `kernel_request_timeout`, `kernel_connection_lost` or `kernel_access_unconfirmed` (the kernel could not read the caller's process just now; the grant has not changed), retry once with the same `--command-id`. On `kernel_outcome_unknown`, read the state first. When the local connection drops with a request in flight, the CLI probes the kernel once: `kernel_access_ended` means the grant ended during the request; anything else is `kernel_connection_lost`.


MP-08 / MP-10 / MP-11: `kernel_sudo_refused` means the person refused the request;
`kernel_sudo_expired` means no approval arrived before its deadline. Neither
starts elevation or retries automatically. Submit a new request only when needed,
and approve it in an already paired Chariox client. The kernel, not the CLI,
picks the access holder from the Unix peer's host ancestry, so no holder pid is
needed for ordinary use, including from a sandboxed tool shell.

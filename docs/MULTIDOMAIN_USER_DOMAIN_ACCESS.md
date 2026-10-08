# MP-08 / MP-10 / MP-11: user-domain access

MP-08/MP-10/MP-11: the main-based round-2 successor uses local443 / relay86.
See `MULTIDOMAIN_ROUND2_ON_MAIN.md` for the exact imported source heads and
validation limits; earlier allocations below describe their branch history.

Owner decision, 2026-10-05. Local protocol **432**, relay peer **78**. This
contract supersedes the focus-revocation text in earlier multidomain receipts.
The kernel owns it equally on ordinary and managed placements.

Focus grants access. Changing focus keeps the previous agent's grant and gives
the new agent its own grant. Browser and notes loading remain independent.
Each grant retains stable resources the agent touched. A retained agent has the
same operations as a focused agent on its granted resources: text entry, keys,
Tab, clicks, activations, navigation, closure and note edits. Loading another
capability or claiming an unrelated existing tab/note still requires focus.
The 16:35 owner decision permits explicit start/open while retained; an explicit
open grants only the newly created tab. Generation, document, owner and
provider-run checks still apply. Listing metadata does not claim resources;
retained browser/notes inventories show only granted resources.

The kernel reads its existing active turns, prompt ownership/backlog, scheduled
agent prompts, interactions and pending MCP/provider continuations. A provider
turn includes waits on harness subprocesses, tools and subagents. Those waits
retain access. A live but idle provider process does not retain access forever.
Only a fully idle agent starts the expiry window. The default is 30 minutes;
`CHARIOX_USER_DOMAIN_IDLE_TIMEOUT_SECONDS` accepts a positive integer override.
Idle duration uses a monotonic clock. Session end, agent destruction, placement
retirement and explicit revoke invalidate admission immediately. Kernel restart
does not restore runtime authority from browser profiles.

Authenticated terminals use `KernelBrowser` commands `list_grants`,
`subscribe_grants { after, wait_ms }` (maximum 25 seconds) and
`revoke_grants { agent_id }` (`null` revokes all of that owner's holders).
These commands do not need a healthy browser or unlocked Vault and bypass
cached observations. Their `user_domain_grants_changed` snapshot includes a
cursor, agent/session/kernel, resources, grant time, focus and expiry rule.
Non-focused resource use advances the cursor and publishes a value-free notice.
Clients keep the last cursor and resubscribe; responses never include resource
contents, credential values or another owner's grants.

Revocation cancels the admission epoch first, retires actor presence and
refuses in-flight results/commits at the next authority check. The same epoch
fences idle subscriptions; an internal controller cleanup closes their streams.
It cannot undo physical input already delivered. Vault fills still require
focus or the existing human approval path. Protected fields/regions,
observation masking, sensitive-action approvals and App validation/passkeys
apply equally to focused and retained agents. Browser input cannot impersonate
a human App frontend or answer approvals.

MP-08 / MP-11: ordinary text and text-producing key events use the same
Vault-only protected-target check, including password/OTP fields, focused
frames and open nested shadow fields, for both focused and retained holders.
Successful browser results are bound to the exact admission epoch under the
grant lock before resource/subscription registration or inventory projection;
revocation followed by refocus cannot adopt an old call's result into a fresh
grant. A final live cancellation/provider-run check fences returned results.

MP-11: the 16:35 owner decision supersedes review rounds 2–4. There is no
retained-input classifier, listener inspection or keyboard allow-list based on
focus. Every physical event checks its document and grant/run cancellation.
A focus change preserves in-flight ordinary input; explicit revoke, idle lapse
and human takeover still cancel it at the next authority check. Vault requests
retain their additional live-focus authority. Note commits keep an uninterrupted
grant epoch without requiring focus again.

Observation reads, including state, never start/recover a controller or
Chromium, for any caller. A stopped/unavailable browser returns
`browser_unavailable`; explicitly start/open it to recover. This applies after
human stop, controller loss and Chromium loss with a live controller. Explicit
start/open uses the same grant authority while focused or retained, with no
grant mutex held across startup I/O. Profiles do not restore expired grants.

MP-08 / MP-11: #882 refusal mapping stays `not_focused_agent` for capability
loading/unrelated new-resource claims, `sensitive_requires_focus` for Vault or
other focus-required protected operations, and `not_granted` for absent,
expired or revoked grants. Browser unavailability is a lifecycle error, not a
focus refusal. String/MCP errors keep the existing transport envelopes.
Protocol remains 432/78; this correction changes no public serialized shape.

There is no cross-kernel control. Browser windows expose `kernel_id`,
`kernel_name`, `focused_agent_kernel_id` and `reachable_by_focused_agent`; App
views expose the same fields under `access`. A remote/leased runtime MCP call
fails with the execution/window kernels and asks the agent to request focus on
the window's kernel. Shared client `userDomainWindowBadge` supplies:
"Your focused agent can't control this window — focus an agent on kernel <name>".
Only clients consuming these new commands/projections need minimum 432; existing
browser/App/notes/capture behavior keeps minimum 427.

App and capture IDs have explicit resource kinds in the shared grant contract.
Browser-hosted App observations use the browser authority; browser mutations
still cannot impersonate a human App channel. The separately proposed App-agent
MCP adapter remains unimplemented in this source. Visible-region capture remains
human-only and becomes ordinary prompt input only when the user sends it. This
change creates no agent capture API, approval-answer path or cross-kernel bridge.

MP-10 evidence must distinguish unit/fixture checks, native Linux browser
execution, official-provider subagent waits, clients, and fresh-machine parity.
None alone closes an MP item.

MP-08 / MP-10 / MP-11 PR #880 regressions: the opt-in
`kernel-browser-input.browser-test.mjs` compares retained and focused native
keys, typing and activation through element/document/window/root handlers.
It also exercises wheel scrolling, paired cancellation and state reads after
human stop. Opt-in Rust `runtime::kernel_browser_host::native_input_tests`
covers kernel grant boundaries, explicit recovery and immediate revoke/idle
lapse. The two-agent held-turn drill covers task/wake retention, resource scope,
notices and idle subscription revocation. These credential-free checks do not
establish official-provider, client, hosted or fresh-machine acceptance.

## Client consumers (mdgrants)

The owner TUI provides `/access` (or `/access list`) and
`/access revoke <agent-id>|all`, through the ordinary owner-terminal
`KernelBrowser` path. Its live grant feed prints a compact retained-use notice.
The web Access view lives inside the existing Browser dialog; no new rail item
is added. It lists holder, session, kernel, touched resource IDs, observed
retained-use time and the kernel idle deadline, with per-holder and all-holder
revocation. Both consumers share the cursor projection and notice formatting.
Only these grant consumers and cross-kernel badges require 432; existing
multidomain browser/App/notes/capture minima remain 427.

Protocol 432 does not expose a per-holder last-use timestamp. Clients label the
latest notice they observed as **last observed retained use**, show **Not
observed** otherwise, and do not infer use from grant creation, idle transitions
or focus. The live snapshot carries only the latest owner notice, so this is
not a complete activity audit. A complete last-use projection requires a future
coordinator-allocated protocol change and updated wire snapshots.

Reproduce the separate-process client drill after building the shared client
and the kernel with `slot-run`:

```sh
CARGO_TARGET_DIR=/w/cx-mdgrants/target CARGO_PROFILE_DEV_DEBUG=0 \
  slot-run cargo build -p chariox-kernel --bin chariox-kernel
node apps/cli/scripts/user-domain-access-live-drill.mjs /w/evidence/mdgrants
```

It creates an explicit disposable `CHARIOX_HOME` under the evidence state
root, uses owner WebSocket controls and provider-bound MCP calls against the
production kernel, holds a dev-stub turn across focus change, claims a panel
note, verifies retained use and revocation, and cleans its owned process group
and state. It does not claim official-provider, native-browser, hosted relay or
fresh-machine acceptance.

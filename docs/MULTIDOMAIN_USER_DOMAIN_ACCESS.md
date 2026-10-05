# MP-08 / MP-10 / MP-11: user-domain access

Owner decision, 2026-10-05. Local protocol **432**, relay peer **78**. This
contract supersedes the focus-revocation text in earlier multidomain receipts.
The kernel owns it equally on ordinary and managed placements.

Focus grants access. Changing focus keeps the previous agent's grant and gives
the new agent its own grant. Browser and notes loading remain independent.
Each grant retains only stable resources the agent used while focused. A
retained agent can observe its claimed resources and scroll; opening a tab,
starting/stopping the user browser, loading another capability or claiming a
new tab/note requires focus. Generation, document, owner and provider-run
checks still apply. Listing metadata does not claim every resource listed.
Retained browser/notes inventories show only claimed resources.

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
It cannot undo physical input already delivered. Vault fills always require
focus. MP-11 (PR #880 round 4): retained access allows only observation
(state, snapshots, text, notes reads) and scrolling through the scroll/wheel
input path. Every key event, including Tab/Shift+Tab, printable characters,
editable-field Backspace/Delete, shortcuts and paired releases, requires live
focus. Text insertion and every click/activation also require live focus,
including Search controls and ordinary page areas. No page labels or script
handlers can grant retained input authority. Refused input dispatches neither
keydown nor its paired keyup and produces no page handler effects. Tab
navigation is available only while focused. Other mutations, including tab
navigation and closure and note edits, require focus. Focused input retains
its document, cancellation and secret-field/Vault safety checks.

Retained observations never start/recover a controller or Chromium. A stopped
or unavailable browser returns `not_focused_agent` with a request to focus the
agent before restarting. This applies after human stop, controller loss and
Chromium loss with a live controller; saved profiles do not confer startup
authority. Focused/human observations may intentionally recover it. The kernel
checks focus before controller startup and live authority after the handshake,
without holding the grant mutex across I/O, then rechecks focus at dispatch;
focused asynchronous requests carry a live focus cancellation guard through
Chromium startup. Focus changes preserve grants, so cancelled operations may
retry through retained authority.

MP-11: the kernel rejects retained non-scroll input before controller dispatch;
the controller enforces the same rule without DOM classification or listener
inspection. Focused physical input carries live focus authority for the whole
operation, including paired releases onto a sensitive target. Focus loss
cancels that operation without revoking its resource grant; retained retries
are limited to observation and scroll. Every physical event checks document
and cancellation authority.
Critical App effects continue through kernel-owned human validation/passkeys.

MP-08 / MP-11: refusal reasons use the #882 vocabulary: `not_focused_agent`
for operations that require focus/new resource claims, `sensitive_requires_focus`
for every key/text/click input or Vault fills, and `not_granted` for absent, expired
or revoked authority. The host controller preserves `sensitive_requires_focus`
in its existing error-code field. Kernel String/MCP errors carry those reason
markers in their existing error text; their outer transport envelopes stay the
same. Protocol remains 432/78 with no serialized shape change in this correction.

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
`kernel-browser-input.browser-test.mjs` exercises real native form submission,
paired focused navigation and activation/keyboard matrices through the
production controller. Retained clicks, text and every key kind dispatch zero
native input and zero element/document/window/React-root handler effects,
including editable-field Delete and Tab. Focused controls/typing still work;
retained wheel scroll, state and snapshot remain available. The opt-in Rust
`runtime::kernel_browser_host::native_input_tests` additionally exercises the
kernel's live focus and grant boundary, retained no-restart fences and idle
stream preservation. These credential-free native checks do not establish
official-provider, client, hosted, or fresh-machine acceptance.

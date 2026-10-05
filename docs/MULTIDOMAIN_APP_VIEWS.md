# App views without a session

Protocol 418 introduces user-domain App views. The domain follows attachment:
a Room view is attached to a session; a user view has no session, Room, slice,
workspace or provider run. The home kernel owns both. Relay remains encrypted
transport and Cloud remains bootstrap/control plane.

## Identity and authority

An App is addressed by the authenticated owner plus installation id. Opening
allocates a kernel view id and captures the installation's verified active
release generation. A view id is a routing identifier, never a bearer grant.
Every request checks the submitting kernel command's admitted human owner.
Another owner, an unverified relay caller, a provider, or a hosted service
cannot use a view id to acquire its owner's authority. The native channel is
a terminal surface; runtime MCP access is a separate authenticated agent path.
The page supplies only a method and input, never owner, actor, generation,
Room, agent, credentials or approval context.

`OpenUserAppView` returns a view descriptor and verified frontend bundle.
`ListUserAppViews` lists the owner's live instances. `GetUserAppViewFrontend`
re-verifies the active release and refuses a generation mismatch; reopen after
an update. `CallUserAppView` invokes the App channel; `CloseUserAppView` ends
the instance and cancels its in-flight calls. `SubscribeUserAppViews` is a
bounded cursor long poll over the ordinary kernel request transport: it
returns the owner's complete view/interaction snapshot when it changes.
Clients repeat with the returned cursor; no relay-side registry is needed.
Owner decision (2026-10-04): instances remain explicit-close ephemeral state
and end on kernel restart. There is no reconnect grace, automatic eviction or
restart restoration for this prototype; old instance/channel ids are invalid
on restart. Installations and App data retain their existing durable lifecycle. Closing a frontend ends that view,
not the shared installation worker. Disconnect/reconnect does not invent a
new installation or session; clients close abandoned instances explicitly.

## Runtime capabilities

Both domains load only the signed `ui/` of an owner-trusted active release.
The frontend receives its own App's channel; workers keep the same sandbox,
capability checks, durable tool queue, cancellation, validation, Vault and
connection/file grant rules. The backend executes on the kernel host, as it
already does for App tools. No user slice is created.

A user-domain call is a human-view call with no `room_id` in its SDK context.
Room calls retain their exact context and foreground-agent binding. User views
do not inherit session history, collaboration, workspace, Room browser/profile,
agent tools, clipboard reads, arbitrary fetch or direct Vault access. App tool
results are data, not permission. Clipboard/link/file operations still use
trusted kernel offers and explicit human acceptance. `chariox.panel` is a
client presentation request; the native host reports no Room conversation
panel. Protected App actions still require human validation and a passkey.

## Approvals without a session

The same kernel-owned `RuntimeInteraction` and shared pending-interaction store
hold user-domain decisions. An owner with live native views receives decisions
in the user domain even when an unrelated Room is open; otherwise existing
Room routing is retained. A detached decision has an owner and operation
subject, no agent and no session. It uses the same monotonic deadlines, bounded
admission, critical passkey verifier, audit and one-winner resolution lock.
Owner snapshots project routine decisions outside the sandboxed App; existing
owner passkey popup subscriptions also project critical decisions. The empty
session routing field on a detached passkey popup means unattached, not a
synthetic session. Protocol-418 clients route these popup replies through
`AnswerUserDomainInteraction` to the shared terminal answer path. Legacy
`RespondToInteraction` retains its Session response contract and cannot answer
a detached decision. Only an admitted terminal of the owner may answer; neither
the App channel nor runtime MCP can answer. Denial needs no passkey; approval
preserves the existing passkey/remember policy. Expiry/shutdown removes the
pending prompt. A Session is never created just to host an approval.

## Hosts and frontend isolation

A small `AppViewHost` command interface separates presentation from policy.
The existing Room host delegates to the slice/local Room browser controller,
with the existing operation-slot retries, call polling, document cancellation,
recovery, layout and foreground binding unchanged. The user-domain runtime
allocates instances and returns native bundles/channel responses through the
same kernel protocol. The kernel Chromium host implements the presentation seam
for an explicit fallback, through the same owner's existing host controller;
it owns rendering only and gains no installation or permission authority.
No Selkies/noVNC or display transport choice is embedded in this interface.

Native clients must serve the bundle on an isolated per-owner/installation
origin, apply the supplied strict CSP, prohibit top navigation/popups and run
it in a sandboxed frame. UI scripts may run inside that App frame; they must
never run in the trusted terminal document. The bridge exposes
`window.chariox.call(method, input)` and forwards only through the kernel.
The `.invalid` origin is a kernel identity, not an Internet endpoint. Native
hosts must map it to their isolated App asset loader; a web host needs an
isolated origin or equivalent isolation before it renders any signed script.
Never load this bundle as a trusted terminal document or as same-origin
`srcdoc` with `allow-same-origin`. Restrict frame ancestry to the trusted client.
Bind messages to the exact frame window, its App origin and the kernel view
id; recheck after navigation and reject messages from other frames/origins.
The page cannot select another view id or call the terminal's approval API.
App origin persistence is separate from Rooms and the trusted UI. Approval,
Vault and conversation UI stay outside that frame. A TUI projects App data
through its own renderer; if it cannot render the signed frontend it uses a
kernel Chromium fallback. The Cloud native frontend remains a separate flagged
prototype; no replacement display transport is selected here.

## Agent access and migration

MP-08 / MP-10 / MP-11: the owner-approved
[user-domain access model](MULTIDOMAIN_USER_DOMAIN_ACCESS.md) replaces the former
focus-only selector. Focus grants access; task lifetime retains only resources
used while focused. New resources and sensitive actions require focus or the
existing human approval path. Cross-kernel tools refuse control and window
metadata exposes reachability.

The App-agent MCP adapter remains proposed. It must consume the shared kernel
grant authority rather than introducing another selector or focus epoch. An
admission must bind the home kernel, owner, session, agent, exact provider run and
grant epoch, with resource checks before dispatch and revocation checks during
awaits/commit. Browser mutations cannot act through the human App channel.

Proposed tool names in that single table: `chariox_user_app_views_list`,
`chariox_user_app_view_read`, and `chariox_user_app_view_call`, loaded as the
on-demand `user-app-views` runtime capability. List/read project owner instances
and declared App data; read never exports a frontend bundle for execution in
an agent. Call invokes the same installation tool queue with the admitted agent
actor, exact run/turn provenance and no inferred room_id; it does not impersonate
a human view. All three recheck grant authority, resource scope and owner, and none can answer
RuntimeInteractions, fetch Vault material or gain window grants. View ids remain
routing ids, not provider tokens. This is a proposal for the registry/MCP lane,
not an implemented or allocated protocol extension. Keep the tool-name table in one responsibility module and reuse the shared
user-domain authority.

Existing `OpenAppView` and Room tab identities keep their behavior and wire
shape. Room views do not silently move to the user domain. A client opening an
App without a session explicitly chooses `OpenUserAppView`; moving a view
means opening a new instance in the destination and closing the old one. App
backend/data remain installation-scoped; transient frontend state remains
per instance. Human/agent simultaneous frontend editing remains deferred.

## Open questions

- Final display-tier choice, transport and Mac acceptance; Linux kernel Chromium
  is bound through lane kbrowser's 417 seam in MULTIDOMAIN_INTEGRATION.md.
- Implement the proposed App tool-name table over the shared grant service when
  the App-agent MCP adapter lane is allocated.
- Production native web origin provisioning and mobile/TUI frontend rendering
  policy. Cloud prototype b64dacb3 stays off by default.
- Kernel browser profile selection, display transport and App multi-interaction
  remain owner decisions outside this lane.

## First integration

See MULTIDOMAIN_INTEGRATION.md. Native remains the default; optional
`host: "kernel_browser"` binds the verified frontend to the same owner's kernel
Chromium, with an optional transport-neutral tab/generation reference. The shared
Room/Chromium host trait only presents verified bytes. Instance creation and App
call/approval authority stay in the shared user-domain runtime; the former native
host's duplicate registry wrapper is removed. App instances are never restored
with ordinary browser tabs. Focused browser mutations cannot impersonate a human App
channel; the proposed focused App tool table still awaits its registry adapter.

## Mutating request recovery

The shared OSS kernel client waits for a slow `OpenUserAppView` or
`CallUserAppView` without replaying it. These owner-authorized requests bypass
transport result caching and have no request-ID receipt: repeating an open
allocates another instance and repeating a frontend call can mutate App state
again. A lost answer after a write reports non-retryable `outcome_unknown`.
List the owner's views to reconcile a lost open; do not retry a mutating call
until the App's own state establishes its outcome. Pre-write transport failures
retain their ordinary classification. The web client's automatic recovery
allowlist already admits reads only; native/App bridge calls use no timeout
retry option. Host offers retain a dedicated one-use terminal acceptance path
for both Room and detached decisions; generic interaction answers may decline
but cannot accept host payloads.

### Reviewed kernel-browser lane integration

The integrated host includes lane kbrowser's reviewed follow-ups through
`2b7268fd9`: mixed native/internal tabs restore to safe URLs, browser operations
use the shared Vault observation barrier, and focused-agent and terminal actors
retain their cancellation, document-binding, takeover and disconnect fences.
App-view generation binding and explicit-close ephemeral restoration remain in
this adapter. Browser input from an agent cannot impersonate the human App
channel. The local protocol remains 418; these internal host APIs do not allocate
another serialized request version. Screenshot capture remains on its separately
allocated 425 branch and must reuse this protected host path.

MD-stack integration: the unreleased feature allocation is folded into local
protocol 427 (relay peer 74). This union and its shape/hash guards supersede the
per-feature versions described during development above.

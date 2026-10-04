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
Instances are transient and end on kernel restart; installations and App data
retain their existing durable lifecycle. Closing a frontend ends that view,
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
recovery, layout and foreground binding unchanged. The client-native host
allocates kernel instances and returns bundles/channel responses through the
same kernel protocol. A future kernel Chromium host implements the same seam;
it owns rendering only and must not gain installation or permission authority.
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
kernel Chromium host once available. This lane supplies the kernel protocol,
not a web frontend or a replacement display implementation.

## Focused agent access and migration

Only the user's currently focused home-kernel agent may discover/read/operate
user-domain views through runtime MCP tools loaded on demand. Focus must be
checked at discovery and again at dispatch, so a previously loaded tool loses
access immediately when focus moves. Cross-kernel access is out of scope.
This lane does not infer focus from a Room's foreground agent or turn an App
view id into a provider token. The broader user-domain registry/focus lane
must bind those on-demand tools to these owner-scoped kernel instances.

Existing `OpenAppView` and Room tab identities keep their behavior and wire
shape. Room views do not silently move to the user domain. A client opening an
App without a session explicitly chooses `OpenUserAppView`; moving a view
means opening a new instance in the destination and closing the old one. App
backend/data remain installation-scoped; transient frontend state remains
per instance. Human/agent simultaneous frontend editing remains deferred.

## Open questions

- Final kernel Chromium host interface and display tiers (lane kbrowser, 417).
- The broader user-domain registry's canonical focused-agent selector and
  on-demand runtime MCP names; access must follow focus without window grants.
- Native web origin provisioning and mobile/TUI frontend rendering policy.
- Reconnect grace/automatic eviction policy for abandoned native instances;
  current lifetime is explicit close or kernel restart, bounded per owner.
- Whether views should be restored after kernel restart; current protocol
  intentionally invalidates old instance/channel identities.
- Kernel browser profile selection, display transport and App multi-interaction
  remain owner decisions outside this lane.

# Computer mode on a macOS kernel host

Status: M0 implemented but unattended, 2026-10-07. Lane `cu/macos-m0`, based on
`e325afa58`. The disabled standalone helper and regression fixture compile and
pass fake-source checks. M0 includes an explicit owner-selected real-window
path. No live capture, AX or CGEvent drill has run; owner evidence remains
required. Linux owns the shared Computer contract; kernel integration and the
remaining Mac adapters follow that contract in M1-M5.

## Decision and dependencies

Use one small signed native helper, supervised by the kernel in the logged-in
user's Aqua session. It captures the explicitly selected Mac surface with
ScreenCaptureKit, observes AX targets, and executes kernel-admitted CGEvent
input. Protected frames enter the multidomain display pipeline. The user sees
the same surface the agent observes through ordinary encrypted kernel/relay
events. There is no Selkies, noVNC, separate desktop server or Cloud media proxy.

Browser mode keeps its structured controller and private browser profile.
Computer mode covers native applications, browser chrome and recovery on the
selected host surface. Both use the kernel's actor ledger, input ownership,
grant admission, protection barrier, cancellation and action receipts. Neither
the helper nor a codec may grant access or resolve a RuntimeInteraction.
For the Chariox browser, bind Computer's window to the existing supervised
browser process and tab registry. A mode switch keeps that browser/profile and
maps its structured viewport to the native surface; it never opens another
browser. User-selected unrelated browsers are native Computer targets only.

Read the following contracts together:

- [Browser and Computer are two modes](BROWSER_COMPUTER_USE_END_TO_END_PLAN.md#browser-and-computer-are-two-modes-not-two-implementations)
  supplies the shared authority and actor requirements. Its older Selkies
  recommendation does not apply to this requested multidomain Mac design.
- [PR #900](https://github.com/charioxai/chariox/pull/900), head
  `6dde21a8c`, supplies `MULTIDOMAIN_KERNEL_BROWSER.md`,
  `MULTIDOMAIN_USER_DOMAIN_ACCESS.md`, `MULTIDOMAIN_REFUSALS.md`,
  `MULTIDOMAIN_SCREENSHOT_CAPTURE.md`, and the round-2 integration record.
  The access amendment supersedes older focus-change revocation descriptions.
- [PR #893](https://github.com/charioxai/chariox/pull/893), head
  `a28ac9993`, supplies `MULTIDOMAIN_NATIVE_CAPTURE.md`,
  `MULTIDOMAIN_KERNEL_BROWSER_DISPLAY.md` and
  `MULTIDOMAIN_DISPLAY_PERFORMANCE.md`. Capture, bounded encode, credits,
  dependent video, exact tile repair and client presentation stay shared.
- [Shared Computer fixtures](SHARED_COMPUTER_INPUT_FIXTURE.md) supply the
  Unicode accelerator regression, bounded holds and physical cleanup oracle.

The requested base and locally available `main` have no `MULTIDOMAIN_*.md` files;
the documents above were read from the available PR branches without checking
them out or merging them. Both PRs were open when inspected. Linux display
performance remains RED and Mac native capture is unsupported there. This plan
claims neither merged dependencies nor native execution evidence.

## Authority, scope and access

The personal Mac desktop is an owner-scoped user-domain Computer resource on
this kernel. Room membership grants the Room's environment, never the owner's
Mac desktop. A future Mac-backed Room environment must be explicitly bound by
its owner and use the existing Room placement contract, without weakening
user-domain isolation. Version one covers the owner desktop only.

The owner explicitly enables Computer mode and selects a surface. TCC consent
alone does not enable it. The kernel derives user and actor from authenticated
terminal or admitted local provider-run identity. Computer capability loading
uses the same focus and retained-grant service as Browser; loading Browser
does not silently load Computer. Focus changes retain ordinary access to
claimed resources. Active turns, waits and pending continuations retain grants;
fully idle expiry, explicit revoke, retirement and session end cancel them.
Use the shared configured idle deadline rather than a Mac timeout policy.
No execution/window cross-kernel bridge is added. Remote viewers may watch or
take over through authenticated kernel requests; a remote provider run does not
gain local user-domain Computer tools by reaching the relay.

| Surface | Capture scope | Input scope |
| --- | --- | --- |
| Selected window, default | One owner-selected window, bound to app process lifetime and window identity | That visible window and its admitted AX descendants |
| Selected app | Windows of one selected app process, with explicit exclusions | Only admitted visible windows of that process |
| Selected display, opt-in | One selected display with private windows excluded | Visible allowed targets on that display; protected targets remain denied |

Widening a selected window/app scope requires an explicit owner decision and a
new scope epoch. A retained agent cannot use a claimed app to claim the display.

App selection does not allow control of system permission panels, other apps or
Chariox approvals. App popups need fresh window admission; dialogs hosted by a
different process require a new owner scope decision. Hidden or minimized
windows may be observable through a supported capture filter, but cannot receive
coordinate input. Window destruction, app restart, display removal, resolution
change, Space transition or uncertain visibility invalidates the binding.
Do not silently switch to whole-display capture when a narrow scope fails.

There is one physical input seat per logged-in Mac user. All native mutations
serialize on that seat, including operations aimed at different apps or
displays. Read-only observations may overlap after protection checks. Structured
Browser operations retain their normal ordering, but operations that change
native focus must join this seat lock. This is an OS constraint below clients,
not an alternative actor policy.

## Capture and display

Implement the display lane's private `start`, `subscribe`, `sample(afterSerial)`
and `close` source contract in a macOS adapter. The Linux owned-Xvfb capability
continues to reject a foreign desktop. A Mac source needs a separate
kernel-issued scoped capability backed by explicit owner selection and live TCC
state. Never make a PID, display string or caller-supplied `owned` flag sufficient.

Use `SCShareableContent` and `SCContentFilter` for display, application or
desktop-independent window capture. Disable audio and microphone capture.
Exclude the helper and Chariox native windows by verified process/bundle and
window identities. Register browser-hosted Chariox viewer, Vault and approval
windows separately; excluding every Chrome window would also hide work targets.
Do not depend on window titles or `NSWindow.sharingType` as a confidentiality
boundary. Apple's [capture sample](https://developer.apple.com/documentation/screencapturekit/capturing-screen-content-in-macos)
demonstrates app exclusions, stream configuration and complete sample buffers.

Start with 30 Hz motion, optional 60 Hz after measurement, and a shallow
`queueDepth` of 3. A dedicated serial capture queue holds one latest pending
sample plus one encode in flight. Drop superseded raw samples before encode;
once a dependent packet has advanced the codec, deliver its chain or reset to
a keyframe. Credits, negotiated bitrate, bounded packets and slow-viewer
retirement remain display-pipeline policy. Static surfaces send no duplicate
pixels; renew control/health independently of repaint.

Accept complete samples only. Preserve capture timestamp, source generation,
geometry revision and protection revision. `SCStreamFrameInfo.dirtyRects` gives
changed areas, including moved content; treat it as a damage hint. Union and
clip hints, compare immutable pixels, and periodically verify the full protected
surface before claiming exact settled repair. Missing metadata, skipped samples
or a changed binding require full readback and a new base. Damage must never
decide whether a protected region exists. See Apple's
[damage metadata](https://developer.apple.com/documentation/screencapturekit/scstreamframeinfo/dirtyrects).

Separate surface points, native pixels and viewer coordinates. Store the
selected display's global point origin, capture content rectangle, content scale,
native pixel dimensions and an explicit forward/inverse transform. Mac display
origins can be negative and mixed displays have different scales. Map a viewer
hit through its letterbox/crop and captured pixel position into Quartz global
points; never multiply an already-native Computer coordinate by Retina scale
again. AX global-point bounds use that same transform. Version one selects one
display, not a stitched atlas. Moving a window between 1x and 2x displays changes
the geometry revision and cancels old input. Do not force Mac desktop resolution
or stretch it into the browser adapter's hardcoded 1280x800 CSS geometry.

Use SDR sRGB as the shared output contract, with explicit color conversion from
wide-gamut/HDR sources. Keep an immutable protected BGRA/RGB surface for exact
PNG/tile repairs and agent screenshot/OCR. Feed a protected `CVPixelBuffer` to
the encoder attachment; use VideoToolbox H.264 when negotiated and available.
Set realtime operation and disable frame reordering, bound pending work, and
emit the shared decoder's codec configuration/keyframe format rather than a
Mac-only wire format. Apple's
[hardware selection key](https://developer.apple.com/documentation/videotoolbox/kvtvideoencoderspecification_enablehardwareacceleratedvideoencoder)
allows acceleration when available; inspect the actual selected backend before
reporting it as hardware. Retain portable software/PNG fallback within the
same budgets. HEVC is deferred until the shared viewer contract needs it.
Protection precedes encoding, including conversion and any hardware submission.
This path may reduce copies; it is not an end-to-end zero-copy claim.

Capture with `showsCursor=false`. Publish the observed human cursor position
and agent intended pointer separately through the shared actor projection.
The viewer draws actor names, colors and action status above video. A small
local helper overlay may show the agent pointer and Stop control; it must be
excluded from captured pixels. Planned session/HID CGEvent mouse input moves the
real system cursor. M0 prefers AX caret placement for text clicks; its scoped
HID fallback can move the system cursor. No separate automation cursor is claimed.
There is no second independently clickable Mac cursor.
Use a labeled arrow for the human cursor initially; exact cross-app cursor shapes
need separate validation. Cursor-only movement updates presence without a frame.

## Input and Unicode safety

CGEvent handles move, click, double/right click, bounded drag, two-axis scroll,
keys and bounded chords/holds. Translate the shared actions into Mac events;
keep key-down/up pairs inside one cancellable Action. No public persistent
down/up command or raw CGEvent API is introduced. Use explicit pixel scroll
units and tested direction conversion, independent of the human's natural-scroll
preference. Physical event posting does not prove application completion; observe
the resulting target/frame and report failure or uncertainty without replay.

Immediately before every event or bounded text chunk, check the grant/run epoch,
Action cancellation, seat ownership, helper generation, source/geometry epoch,
visible target, focused app/window/AX element, protection policy and secure-input
state. Recheck after dispatch. Only an admitted activation Action may change
focus; typing never reacquires focus after a mismatch. Untrusted AX data, OCR
text or application content cannot author a grant or answer a Chariox approval.

Text and shortcuts are distinct operations. Use
[`CGEventKeyboardSetUnicodeString`](https://developer.apple.com/documentation/coregraphics/quartz-event-services)
with validated UTF-16, bounded grapheme-aware chunks and no inherited Command,
Control or Option flags. Reject invalid input and unsupported controls; newline
and tab are deliberate controls only when the shared text contract permits them.
Do not split surrogate pairs, normalize the user's text, remap the system layout,
or fall back to clipboard/AppleScript. A Unicode payload still travels with a
physical virtual key code. Admit only a tested inert text-event strategy;
untranslated input must fail before a refresh, media key or shortcut can occur.
Known key chords use explicit Mac key mappings and the ordinary target fence.

The Linux fence is a behavioral requirement, not an X11 implementation to port.
Reuse its oracle: composed/decomposed text, emoji, CJK, German/non-US layouts,
long input, multiline, dead keys, IME state, cancellation, and an input that would
otherwise trigger refresh must leave the expected text without navigation or
focus loss. Prototype CGEvent Unicode in fixture apps before enabling it for
general targets. If an app rejects Unicode events or consumes their keycode as
a shortcut, return unsupported. AX value assignment is a separate future
structured action with the same checks, never an invisible fallback.

Refuse any agent action into password, OTP, payment-secret, Vault, Keychain,
passkey or OS authorization UI. Refuse keyboard input when
`IsSecureEventInputEnabled()` is true, even if an AX element appears ordinary;
pause control and withhold observations until a fresh safe binding exists.
Never disable secure input to make automation work. Apple's archived
[TN2150](https://developer.apple.com/library/archive/technotes/tn2150/_index.html)
explains that secure input can suppress event interception across processes.

CGEvent posting and AX focus queries do not provide X11's server-grab atomicity
between target validation and delivery. Rechecks narrow the race but cannot undo
one already delivered event. Therefore version one has no native Computer Vault
secret insertion. Use the existing protected Browser Vault path for supported
web fields, or let the human enter secrets with capture paused. General native
typing stays experimental until adversarial focus-change tests pass; unknown
editable targets are refused rather than treated as safe from OCR alone.

## Human takeover and held input

A listen-only session event tap observes local activity without suppressing or
modifying human events. Tag helper-originated events with a private source marker
and validate source PID/lifetime to avoid counting its own input as human input.
The marker is a classification hint, not authorization. Unclassified external
input, movement, scroll or keys pause the agent immediately. Retain only event
class, time and necessary pointer coordinates, never human keycodes or text.

The helper sets a local dispatch fence before notifying the kernel. The kernel
projects the physical owner as a human actor, cancels the active Action, fences
queued mutations and settles ownership through the existing takeover machinery.
Remote-viewer human input uses that same ledger. Resume requires explicit human
release and fresh target admission, not an idle timer or agent-driven refocus.
Physical-human ownership is conservative when local and remote humans conflict;
the existing owner-only release rules still apply to remote takeover.

If the event tap is disabled, times out or lacks permission, control pauses.
Secure input also pauses control because keyboard activity may be unobservable.
Do not assume Accessibility always authorizes every listen-only tap on every OS;
validate `CGPreflightListenEventAccess` and the actual chosen tap in the signed
prototype. If Input Monitoring is additionally required, report a separate
owner opt-in and keep control unavailable until granted. No automatic request.

Reject new holds/chords when foreign keys or buttons are already held. Track
only helper-owned presses and release them on cancel, revoke, timeout and orderly
exit without lifting the human's keys. The kernel retains a value-free in-flight
press ledger for recovery. Fatal helper death may leave uncertain native state;
Mac cannot safely resolve that by killing the target app or resetting all keys.
Pause the seat, attempt verified helper-owned release only when permissions and
foreign-hold checks permit it, otherwise require the human to clear the state.
No new Action can start until reset succeeds. Fatal-death recovery must pass
before long holds are enabled.

## Structured targets and observation protection

Use `AXUIElement` on the scoped app/window, with a bounded tree walk and per-call
messaging timeout. Return opaque target references, role/subrole, safe label,
enabled/focused state, supported actions and global-point bounds. Do not expose
raw AX handles, process-wide trees, arbitrary attributes or AX scripting to
agents. Use `AXObserver` notifications to invalidate refs on focus, destruction
and layout changes; revalidate on use because notifications are incomplete.
Bind refs to process start identity, surface/helper generation, geometry and
observation revision. Prefer supported AX press actions for observed controls
through the same Action service; coordinate fallback still needs live hit testing.
Apple's [Accessibility API](https://developer.apple.com/documentation/applicationservices/axuielement)
is the native targeting layer.

Do not read secure fields' AX values, selections or children into observations.
Drop their labels/descriptions if protection cannot distinguish metadata from
secret-bearing content. Apply the shared Vault policy and retired-value scrubber
to all permitted labels, OCR, errors and screenshots. Treat AX as application
supplied data, not proof that an app faithfully marks every sensitive field.

Use local Vision `VNRecognizeTextRequest` on a protected immutable frame only.
Return text, confidence and bounded boxes tied to that frame revision, convert
Vision's bottom-left normalized coordinates to the shared top-left surface
coordinates, and report ambiguity rather than guessing a target. OCR provides
targets when AX is absent, never an exemption from input/protection checks.
See Apple's [text recognition](https://developer.apple.com/documentation/vision/recognizing-text-in-images).

Private UI exclusion and secret masking happen before screenshots, Vision,
encoding, thumbnails or agent image delivery. Kernel-issued exclusion IDs and
the helper's own window set are mandatory, including newly created overlays.
Fence publication while exclusions or protection change; flush capture buffers,
encoder references, pending tiles and old observations, then bootstrap a fresh
protected base. Excluding an overlay alone can reveal the app underneath it;
replace its footprint with opaque pixels when that underlying content is also
private. A discovered exclusion is not a retrospective cure for a leaked frame.

AX can be missing or race compositor pixels. When sensitive geometry or the
shared protection provenance is unknown, mask the whole affected window, or
withhold the entire frame if its bounds are uncertain. App scopes cannot claim
universal secret detection, and whole-display sharing is especially broad.
Use a clean dedicated workspace for the first grant-backed drill. Refuse rather
than run OCR on a raw frame to decide whether it should have been protected.
Browser native capture keeps the existing document/Vault fence; protected
browser content uses its protected CDP fallback instead of broadening Mac capture.
No raw frames, AX trees, text, clipboard contents or secrets go to logs, durable
action history, crash reports or relay/Cloud stores. Native clipboard support
is deferred; existing Linux clipboard behavior is not silently applied to the
human's global pasteboard.

## TCC, binary identity and owner setup

Ship a minimal `Chariox Computer Helper.app` with a stable bundle identifier,
installation path and signing requirement. This one native executable performs
ScreenCaptureKit, AX, event posting and local activity observation. Give it
Screen Recording and Accessibility, rather than granting the changing Rust
kernel, Terminal, Node, Python or a provider executable. The kernel keeps policy
and supervises the helper; the helper holds OS permissions and exposes only the
bounded private adapter. No root daemon, system extension or sudo is needed.
Use Swift with a small AppKit menu/Stop control and native framework calls;
avoid an embedded scripting runtime or a separate permission broker.

TCC attribution depends on responsible code and launch context; putting a binary
in an app bundle does not prove independent attribution. The first prototype
must verify the exact shipping launch route, including launch from a CLI kernel.
Prefer LaunchServices launch of the standalone helper app if direct spawning
attributes permissions to the parent. Kernel ownership then means authenticated
process binding, liveness and stop, not an assumption about OS parent PID.
If the chosen route names Terminal or the kernel in System Settings, stop and
fix packaging/launch identity before requesting broad grants. Keep a single
permission-bearing executable, without inherited child permission assumptions.

Use Developer ID Application signing with the owner's Apple Developer team,
hardened runtime, secure timestamps, notarization and a stapled ticket for
distribution outside the App Store. A `.pkg` would also need Developer ID
Installer; start with an app bundle. The owner must provide Developer ID and
the protected signing/notarization workflow. Do not generate replacement keys,
export certificates/private keys or place them in source, evidence or runtime
state. Follow the key-retention protocol before later key work. No signing or
Keychain inspection occurs in this lane. Apple's
[Developer ID guide](https://developer.apple.com/help/account/certificates/create-developer-id-certificates/)
and [distribution signing guide](https://developer.apple.com/documentation/xcode/creating-distribution-signed-code-for-the-mac/)
describe the release requirements. Signing/notarization do not grant TCC access.

At a later owner-attended setup, the owner must:

1. Open the signed helper and explicitly choose Enable Computer mode with the
   displayed target scope. Review the helper identity before granting access.
2. Allow the helper under System Settings, Privacy & Security, Screen Recording
   or Screen & System Audio Recording, depending on macOS. Enable Accessibility
   for that same helper. Complete macOS authentication personally if requested.
3. Quit/reopen the helper if macOS requires it, then confirm a safe target and
   test the local Stop control. Grant Input Monitoring only if the validated
   activity observer requires it and the owner accepts that separate permission.

This is the initial setup, not a promise of only one OS click forever. OS updates,
signing identity changes and capture reminders can require renewed consent.
For a first implementation target macOS 14 or newer; prove actual availability
with SDK/runtime guards, then choose supported OS versions from evidence.
Readiness uses nonprompting checks in the eventual helper:
`CGPreflightScreenCaptureAccess`, `AXIsProcessTrustedWithOptions` with prompt
false, and applicable event-access preflights. Only the owner's explicit setup
gesture may invoke a request API or open permission settings. Apple's
[AX trust check](https://developer.apple.com/documentation/applicationservices/1459186-axisprocesstrustedwithoptions)
documents its prompt option. No permission-check executable is run now.

## Helper lifecycle, revocation and stop

Keep responsibilities small: kernel Computer service handles admission, actor
ledger and scheduling; native capture handles filters/geometry/protected pixels;
native targets handle AX/Vision; native input handles events/owned presses;
supervisor handles process identity and health. A thin helper coordinator wires
these together. Reuse the display encoder attachment and shared transport rather
than adding a second viewer stack.

Version one permits one helper owner per logged-in seat. A competing kernel
cannot adopt an existing helper or take the seat from its live owner. Connect
over a private local socket or inherited pipe, never a network listener. For LaunchServices,
use a 0700 rendezvous directory and a one-use 0600 bootstrap file outside source;
do not put authentication bytes in argv. Bind both ends to the expected UID,
kernel/helper PID and process start identity, and verify the installed helper's
code-signing requirement. Production helper admission must authenticate the
expected kernel too, not merely any same-UID caller. Unsigned drill pairing needs
an explicit drill-only allowlist. M0 admits the signed sibling public fixture
in fixture mode. Owner mode and window discovery admit only `com.apple.TextEdit`,
with that bundle ID and the running PID's SecCode satisfying
`identifier "com.apple.TextEdit" and anchor apple`. Transport discovery alone grants nothing.
Same-UID arbitrary code can already attack an unsandboxed user session; this
helper is an admission boundary, not a sandbox against a compromised owner.
Requests carry a cancellable kernel operation and current epochs, with bounded
binary frame output. The helper has no tool HTTP endpoint or provider credentials.

Use a short kernel liveness lease, initially a one-second heartbeat and two-second
expiry, subject to measured scheduling tolerance. Disconnect, expired lease,
kernel exit or Stop immediately fences native input and capture, releases owned
presses when safe, stops SCStream and clears buffers. The kernel cancels Actions
and retires subscriptions on helper crash; completed mutations are never replayed.
Crash recovery gets a new helper/source generation, fresh AX references and a
keyframe. Observation never starts a stopped helper. Require explicit owner
restart after crash or uncertain input; retain deliberate takeover across helper
recovery. Full kernel restart restores no grants or input ownership.

Poll nonprompting permission health while active and handle capture errors, AX
denial, tap loss, session lock/logout and sleep immediately when reported.
Revoking Screen Recording withholds frames; revoking Accessibility or observer
permission pauses control. Wake/unlock requires fresh scope and geometry checks.
Do not retry input, request consent or use another capture API after revocation.
TCC changes are not guaranteed synchronous with a frame callback; final commit
checks and short health polling bound detection, and the live drill measures it.

Provide Stop in the helper's local menu/control and in each authenticated viewer
and TUI projection. The local control acts in the helper before a kernel round
trip and remains usable during relay loss. An optional tested global shortcut
is supplementary, since secure input may prevent event taps from seeing it.
The kernel Stop route revokes Computer grants and subscriptions, cancels queued
Actions and fences the helper. It remains usable without a healthy capture
stream or unlocked Vault. No agent action may dismiss or re-enable Stop.

## Protocol reservation

Reserve the logical Computer surface/adapter change for coordinator allocation;
numeric local-daemon and relay versions are UNALLOCATED. This document allocates
none and changes no wire DTO, snapshot, hash or client minimum. The coordinator
must reconcile the Linux Computer contract with the multidomain display union
before assigning versions. Existing PR numbers and historical protocol versions
are not allocations for this work.

The shared implementation needs to express:

- Owner-scoped surface identity/kind, host kernel, source generation, point/pixel
  geometry and transform revision. Desktop/window/app targets cannot masquerade
  as browser tab/document IDs or rely on the browser's fixed CSS viewport.
- Capability/readiness and safe refusal status for missing TCC, unsupported
  targeting, secure input, private content, stale scope and uncertain reset.
- Protected observations and typed Actions bound to surface/observation,
  grant and geometry epochs; structured AX/OCR refs expire with those epochs.
- Existing actor/pointer/takeover projections extended to a physical seat,
  plus start/stop/revoke lifecycle through normal kernel requests.
- Caller-bound display attachment and protected frame delivery using existing
  credits, sequence/base validation, codec negotiation and encrypted events.

Prefer adapting existing shared Computer Actions and display metadata. A private
OS adapter or encoder needs no wire change by itself. When serialized shapes
or terminal semantics change, the implementation PR must take the coordinator's
assigned version, update Rust/shared-client snapshots and hashes, raise only
dependent client minima, and add a focused local/relay drill. Relay stays opaque;
do not add plaintext AX, capture or input handling there. Extend the existing
typed refusal contract without exposing target existence or app/error contents.

## Validation: real live drills (owner rule, 2026-10-06)

Each Mac PR runs real live drills of the capability it adds, on the real Mac with real apps, never fixture-only. M0, for example, runs the owner-granted signed helper capturing and driving a real app window. The **feature acceptance gate**, before anything is staged for the owner, is the full set of real live drills of exact user scenarios across M0–M5:
- real Mac apps (Finder, TextEdit/Pages, Safari/Chrome on real sites, System Settings refusal);
- real official providers on real accounts;
- the real web app in a desktop browser at DPR 2, and the real TUI;
- the hosted relay;
- realistic durations, including sleep/wake and lock.

Fixture apps and fake capture sources are regression checks only. Intermediate PRs stay feature-disabled and unstaged. Nothing is staged for the owner until M0–M5 are complete and the full gate passes.

## PR sequence and size limits

Linux defines and validates the shared seat/surface/grant contract first. Rebase
the Mac implementation only when that contract and the display union are agreed;
do not copy a competing Mac actor service into this design branch. Sizes below
are rough changed source/test lines, excluding generated lockfiles and evidence.
Split at responsibility boundaries when a PR exceeds its budget.

| PR | Size | Deliverable and merge proof |
| --- | --- | --- |
| M0, feasibility helper | M, 800-950 | Native regression fixture and owner-selected real-window helper, disabled by default, signed launch identity, explicit owner setup, one scoped SCK frame and benign AX/CGEvent round trip in a real app; owner-attended evidence required |
| M1, shared adapter and lifecycle | M, 400-700 | Implement Linux-approved Computer adapter registration, private helper pairing, health/Stop/revoke and stale epochs; focused fake-backend tests and actual crash/denial proof |
| M2, capture attachment | M, 500-800 | SCK filters/exclusions, HiDPI transforms, damage and protected exact repair; shared viewer shows public fixture pixels over encrypted kernel/relay events |
| M3, input and takeover | M, 500-800 | Scoped mouse/scroll/keys/Unicode, seat serialization, local-human fence and owned reset; Linux-equivalent physical fixture and adversarial focus/takeover/death cases |
| M4, targets and protection | M, 400-700 | Bounded AX refs, Vision fallback, secure/private refusal and revision invalidation; synthetic password/canary, opaque AX, OCR ambiguity and no-leak proof |
| M5, VideoToolbox and release | M, 350-600 | Shared codec attachment with software fallback, signed/notarized install/upgrade, owner onboarding and full Web/local-TUI/remote-TUI matrix; functional and performance statuses reported separately |

M2/M3 must remain feature-disabled until M4's protection gates pass. Every
implementation PR includes its own focused tests, rollback and exact ownership
cleanup. Allocate protocol changes in the shared contract PR rather than one
number per OS module. Local `[skip ci]` checkpoints precede final-head review/CI
when the feature is fully ready. M0 and its round-2 review-fix checkpoint are
published as draft PR #920, with GitHub CI skipped.

## Minimal feasibility prototype, implemented M0 and owner-attended gates

M0's standalone helper uses supported AXPress for buttons and a bounded numeric
AX vertical-scrollbar value step for scroll areas. Text-area/field clicks query
AXRangeForPosition at the requested global point, focus the element, assign a
zero-length AXSelectedTextRange at the returned character range's start, and
read back the selection. This operation reads numeric range metadata only,
never document text. Without a requested point, it uses the center of the
element/window/display intersection. A fully invisible target still refuses.
Only an unsupported or unavailable AX position range selects the session/HID
CGEvent click fallback. Malformed ranges, permission errors, failed focus or
selection mutations, and unproven readback never trigger a fallback or retry.
Text typing retains per-PID CGEvent delivery and its 20-UTF-16-unit limit.

Every path retains PID/window, frontmost, focused-window, secure-input and live
hit-test fences. Clicks retain the geometry snapshot and actual point across
resolution, mutation, posting and observation. An owned HID mouse-up retains
the original process lifetime and AX/CG window ownership checks while allowing
geometry changes, or reports unresolved owned input requiring owner reset.
AX receipts prove a synthetic button-counter increment, numeric scrollbar
increase, or selection-range readback. HID dispatch alone remains unproven.
See `apps/macos-computer-use/OWNER_RECHECK.md` for the single pending owner
recheck. The unattended NSTextView drill reached the public helper's permission
gate but could not use the owner's Accessibility grant. Real TextEdit caret
placement, live HID delivery and fatal input recovery remain unproven. No
session-tap observation or self-tagging is claimed. Unicode typing refuses an
entire overflowing string without splitting a grapheme or surrogate pair. The
broader steps below remain later acceptance gates, not instructions to run
capture unattended.

1. Build the smallest native helper plus public fixture with ordinary and secure
   text fields, buttons and moving color patches. Use external compiler output
   and disposable state. Compile/fake-source checks require no TCC calls.
2. Have the owner supply the protected Developer ID workflow and install the
   signed helper at a stable path. Verify the actual CLI-kernel launch attribution
   before the owner grants permissions. No grant to Terminal/provider tooling
   is an acceptable shortcut. Record only public signing identity and hashes.
3. After the explicit owner setup, capture one fixture window, then the selected
   fixture app and finally a clean display if the owner opts in. Check Chariox
   exclusions, opaque private footprints, cursor separation, 1x/2x geometry,
   negative display origins and window movement. Inspect exact protected pixels.
4. Drive a benign click, scroll and Unicode edit through a disposable kernel's
   normal Action/grant path. Compare AX targets with OCR fallback and the actual
   protected screenshot. Observe physical human input cancelling the Action;
   verify secure fields/input refuse, focus changes abort, and Stop works without
   relay connectivity. Establish owned-release behavior on fatal helper death.
5. Attach the existing multidomain presenter through the disposable encrypted
   relay path. Measure capture-to-publication, input-to-visual-ack P50/P95,
   authoritative presented fps, exact restore, bitrate and peak owned memory.
   Compare VideoToolbox with fallback separately. Reuse Linux's visual-ack,
   motion and exact-repair targets; hardware availability does not establish PASS.
6. Revoke permissions by the owner's manual System Settings action, crash the
   helper, lock/sleep/wake, lose the relay and reconnect. Assert no input replay,
   no stale pixels/targets/grants and bounded stop detection. Gate broad use on
   those results, signed upgrade identity and the supported OS matrix.

The first live gate needs the owner's grants and Developer ID. Until then Mac
feasibility is UNPROVEN. TCC attribution, reliable local-activity observation,
Unicode inertness, compositor/AX protection races and fatal held-input recovery
are acceptance questions, not assumed platform guarantees. Broader display
scope stays opt-in; unsupported targets remain refused.

Store public-fixture screenshots and receipts under
`/Users/miguel/.codex/evidence/browser-computer-use-macos/`, never in Git. Use an
explicit disposable `CHARIOX_HOME` outside source for later kernel drills, with
no owner profiles or credentials. Inventory exact drill-owned processes and
directories before removal; do not stop installed kernels or recursively clean
shared state. Retain signing assets and grants unless the owner removes them.
M0 adds the standalone helper, public fixture, fake-source tests, build script
and owner guide. Compile checks and ad-hoc signing have run; real-window drills,
Developer ID signing and TCC attribution remain unproven. This lane touches no
running kernel, Keychain or reviewer state.

M0 has no `LocalDaemonRequest`/`LocalDaemonResponse`, relay-event or terminal
transport shape changes. Its standalone helper is not a client protocol, so
the Protocol Change Rule requires no shared version bump for M0.

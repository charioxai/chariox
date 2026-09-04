# Room provider Browser and Computer drills

### Claude tool transcript resource limits

Tool invocation/result projection is bounded before retention and output
serialization. A turn retains at most 64 unmatched invocations and 256 KiB of
encoded input plus ID/name bytes. Each input is capped at 8 KiB before cloning;
oversized input is replaced by `{"chariox_truncated":true}`. Pending inputs stay
encoded instead of retaining expanded JSON trees. IDs and tool names are capped
at 256 bytes, and projection examines at most 64 content blocks per message.

Results append text incrementally, retaining at most 16 KiB including a visible
`[chariox: tool transcript truncated]` marker. At most 256 result blocks are
examined. No unbounded text-block collection/join occurs. Missing or oversized
identities and exceeded tracking/message budgets produce one transcript-limit
notice per turn. These limits affect transcript projection, not provider tool
execution. Completing a tracked call releases its retained byte budget; turn
reset clears both pending calls and the notice state.

The standalone actual-source test covers oversized input/results, Unicode and
exact-size boundaries, many unmatched IDs, aggregate byte exhaustion, result
correlation, slot reuse and reset. Full kernel integration and live Claude
provider validation remain separate requirements.

These local acceptance cases use an official provider through the kernel's
normal slice-backed agent path. The test driver does not impersonate an agent
with direct MCP calls. Use them before managed-machine validation and public
benchmarks, following `BROWSER_COMPUTER_USE_END_TO_END_PLAN.md`.

### Explicit Colima forwarding for local drills

If the relay port works inside the Colima VM but macOS cannot reach its
published loopback port, the local Room drill can own its forwarding:

```sh
export CHARIOX_ROOM_DRILL_COLIMA_SSH_CONFIG=/absolute/path/to/colima/ssh_config
```

This is opt-in local test infrastructure, not a runtime transport replacement
or a repair of Colima's shared auto-forwarder. The normal kernel/relay/provider
paths remain unchanged. Use the existing trusted Colima SSH configuration.
The helper forwards only the drill-owned slice's assigned kernel, relay,
viewer and provider ports to the same loopback ports inside Colima. It starts
after container creation, so the kernel's initial port-availability check is
not obstructed, and requires SSH confirmation of every listener. A failed
forward waits for in-flight provisioning to settle before cleanup can proceed.

One SSH process uses no shared control socket, no control master and no
persistent connection. Startup failure terminates that process; normal/failure
cleanup closes it before checking every forwarded port for leaked listeners.
Unexpected SSH exit cannot produce a passing cleanup. Source evidence records
the forwarding mode and port numbers, not SSH diagnostics or configuration
contents. The helper never restarts Colima, edits its configuration, or acts on
another task's SSH process. Unset the variable for normal Docker forwarding.

## Select the case

Set these environment variables when running the paired Cloud
`apps/web/scripts/view-route-local-room-tui-drill.mjs` script:

| Variable | Value |
| --- | --- |
| `CHARIOX_ROOM_DRILL_FOCUS` | `web-companion` |
| `CHARIOX_ROOM_DRILL_WEB_REAL_PROVIDER` | `1` |
| `CHARIOX_ROOM_DRILL_PROVIDER` | `codex`, `claude`, or `opencode` |
| `CHARIOX_ROOM_DRILL_MODEL` | explicit model supported by that official provider |
| `CHARIOX_ROOM_DRILL_PROVIDER_MODE` | `browser` or `computer`, default `computer` |
| `CHARIOX_ROOM_DRILL_BROWSER_TASK` | optional `click` or `form`, only in Browser mode |
| `CHARIOX_ROOM_DRILL_BROWSER_LAYOUT` | optional `page`, `nested-frame` or `shadow-root`, only with the form task |
| `CHARIOX_ROOM_DRILL_BROWSER_MUTATION` | optional `replace-field`, only with the form task |
| `CHARIOX_OSS_REPO` | absolute path to the matching OSS worktree |
| `CHARIOX_SLICE_DOCKER_PROVISIONER` | that worktree's `apps/kernel/slice-linux-docker/provision-linux-docker-slice.sh` |
| `CHARIOX_ROOM_DRILL_IMAGE` | prebuilt image matching the runtime source fingerprint |
| `CARGO_TARGET_DIR` | existing component binaries outside repositories |

The runner also needs the existing built TUI/kernel-client and paired Cloud
Web/API outputs, Chrome and Playwright. Reuse task-specific build outputs and
dependency caches. Do not build or install another copy per worktree. Provider
authentication must be valid in the selected default official profile. This
mode consumes real provider usage. Import-first is not supported by the Web
case; automatic kernel-managed account materialization is part of the test.

For physical/TUI-only coverage, use `CHARIOX_ROOM_DRILL_FOCUS=real-provider`
with the same provider/model/mode selection and run the OSS
`apps/cli/scripts/live-room-environment-pointer-click-drill.mjs` script.

## Required observations

Computer mode asks for one coordinate click through `slice_mouse`. Browser
mode adds a fixture button, asks for `slice_browser_find`, then asks for one
`slice_browser_click` with its opaque `field_id`. The Browser case requires
a completed `browser/click` action targeting exactly one tab; a Computer
pointer click cannot satisfy it.

With `CHARIOX_ROOM_DRILL_BROWSER_TASK=form`, the provider discovers the
`Browser sample` input, fills `Chariox form sample`, discovers the submit
button and submits its form using opaque field IDs. The fixture server emits
`BROWSER_FORM_ACCEPTED` only after receiving the expected submitted value.
Acceptance requires fresh, completed Browser fill and submit actions by the
same actor, targeting the same tab in that order. Both TUIs must show both
actions. Web must display the accepted page and remain usable for human input.
This tests form-driven navigation, not the full navigation and lifecycle matrix.

The `nested-frame` layout puts the only form controls inside two same-origin
`srcdoc` frames. The `shadow-root` layout puts them inside an open shadow root,
with no light-DOM copy. Submission navigates the top page; only server-accepted
values produce the success marker. The accepted page returns to the normal
layout so the same human keyboard/selection/scroll checks can follow.
These cases do not prove cross-origin frames, closed shadow roots or stale
references. Each still needs its own acceptance case.

With `CHARIOX_ROOM_DRILL_BROWSER_MUTATION=replace-field`, the provider first
discovers the field, then clicks a fixture button that replaces it. It must try
the original reference once with a sentinel value, receive a stale-reference
error, rediscover the replacement and fill/submit successfully. Acceptance
requires exactly four fresh actions in one tab by the same actor: replacement
click, failed fill, completed fill, completed submit. Both TUIs must show the
failure as well as the successful actions. The kernel history records the
bounded `controller_failure` outcome; bounded provider-tool output separately
must prove `stale_element_reference`. Prompt text and model claims cannot
satisfy that check. A provider without projected tool-result errors cannot
pass this evidence gate.

The page records whether the rejected sentinel ever landed before the correct
fill, and the server emits `BROWSER_STALE_RECOVERY_ACCEPTED` only for a replaced
field with that safety check intact. Web must paint the accepted page before
human takeover. The reduced fixture test deliberately inserts the sentinel,
overwrites it, and verifies the server still rejects acceptance. The recovery
drill is opt-in; adding it does not establish a live provider pass.

Before a paid provider run, check the physical fixture structures with an
already-installed Playwright module and Chrome:

```sh
PLAYWRIGHT_MODULE=/absolute/path/to/playwright/index.mjs pnpm --filter @chariox/cli run test:room-provider-browser
```

This explicit browser integration test never downloads dependencies. The normal
focused suite remains browser-independent; final validation must run both.
The nested-frame case also captures the real controller snapshot and fills and
submits through its returned backend references. Snapshot collection reads
each frame's accessibility tree, shares the existing node budget across frames,
caps traversal at 64 frames and rejects a changed frame tree during capture.
The shadow case also fills and submits through controller references. A
separate real-Chrome case checks document-bound masked input inside a shadow
root with a fixture-only password. These reduced tests do not replace live
provider, Web and TUI acceptance or prove vault-backed credentials end to end.

Both cases check the authoritative provider configuration, Room/slice
membership, agent attribution and an action sequence newer than the pre-prompt
baseline. Web must paint the physical counter change at the canonical
viewport, then human takeover must work in that same environment. Both TUIs
must retain the matching action notice. Subsequent keyboard, selection and
scroll checks exercise the same physical page.

Run focused assertions without a package rebuild:

```sh
pnpm --filter @chariox/cli run test:room-provider
```

The normal CLI package test command includes that focused suite. Repository CI
still runs only at the final reviewed, merge-ready gate.

## Evidence and limits

### Claude tool-result history

Claude stream-json assistant `tool_use` events start a running tool transcript
entry. Matching user `tool_result` events complete the same entry or attach
the actual tool error, preserving the invocation input and tool ID. A rejected
tool is not by itself a failed provider turn. Text result blocks are retained;
image payloads are not copied into this textual projection. Unknown or duplicate
results are ignored, and pending invocation context clears on prompt reset,
settlement, cancellation and runtime restart.

The isolated tests exercise the actual projection source with the existing
serde_json dependency. They do not prove the full Claude event-drain wiring or
a live provider error. The kernel test
`claude_stream_projects_matching_tool_result_error_and_input` and the live
provider/Web/TUI recovery drill must also pass before accepting that coverage.

### Controller reference recovery

Run the real-Chrome controller request tests before the live provider drill:

```sh
PLAYWRIGHT_MODULE=/absolute/path/to/playwright/index.mjs pnpm run test:browser-controller-browser
```

This uses the normal controller request dispatcher and persistent CDP client,
not a mocked browser connection. Each case creates and removes its own temporary
Chrome profile outside the repository, closes the controller and browser, and
requires an already-installed Chrome. It tests replacement of a field in the
top page, two nested frames and an open shadow root; top-page navigation; and
child-frame navigation with an unchanged top document. Loopback HTTP fixtures
also exercise cross-site frames in a verified isolated renderer target and
same-site cross-origin frames sharing the parent renderer. Both must support
discovery, fill and native click. Isolated frame navigation must reject old
references and permit rediscovery, and a reference from another tab must fail
without inserting text. Isolated renderer references include their frame and
document identity because backend node numbers can collide between renderers.

Old references must produce `stale_element_reference` or, after top navigation,
`stale_document_reference`, without inserting text into the replacement. A new
snapshot must discover a usable field and fill it successfully. Detached nodes
are rejected immediately, not polled as temporarily hidden elements. These are
controller-level checks, not proof of provider-led retry, kernel action-history
projection, concurrent navigation during insertion, or managed Linux behavior.

### Controller dialogs

The same real-Chrome command opens alerts, confirmations and prompts through
controller clicks and answers through `browser.dialog`. It checks accept and
dismiss, Unicode and explicit empty answers, absent-dialog errors followed by
successful input, and rejection of stale-document or oversized requests while
the original dialog remains answerable. Dialog events must carry the correct
target and document without exposing messages, defaults or answer text.

Accepting a prompt without `prompt_text` preserves its default. An explicit empty
string clears it. The controller keeps a maximum 2048 UTF-8 bytes of default text
privately until the dialog closes, navigates, detaches or crashes, or the browser
connection resets. Larger defaults require an explicit replacement or dismissal;
they are not truncated or copied into events. Focused CDP-boundary tests cover
cleanup and cross-target isolation.

These tests do not prove provider-led dialog handling, Web/TUI dialog projection,
before-unload prompts, timeout recovery or reconnect while a dialog is open.

Keep the exact source/image fingerprints, provider identity, action ID and
sequence, physical result, Web screenshot, TUI assertions, resource samples,
outer process exit and cleanup ledger. A provider text response or an early
action checkpoint alone is not a passing run. Cleanup must remove the owned
container, volume, temporary state and listeners and pass the fixture-secret
scan. Keep evidence outside repositories under `~/.codex/evidence/`.

The Room drill assigns its home kernel a private `CHARIOX_LOG_DIR` under the
disposable drill root. On failure, before teardown can flood the log with
disconnect retries, it writes `kernel-connection-diagnostic.json`. This captures allowlisted
connection stages and fixed error categories, not raw log lines, tokens, URLs,
Room names, paths, prompts, or provider payloads. Primary and private relay URLs
are compared in memory and recorded only as `primary` or `private`.

Capture scans at most 64 directory entries, selects at most eight daemon log
files, reads at most 64 KiB per file and 256 KiB total, and retains the latest 128
matching events from those reads. Symlinks and non-regular files are ignored.
Missing, unavailable and truncated evidence is explicit. This diagnostic does
not establish that a failed connection recovered. The live slice failure and
the complete provider/Web/TUI acceptance case must still be rerun.

The privacy and bounded-read tests run with `test:room-provider`. To verify the
capture against an already-built kernel without building, starting a slice or
calling a provider:

```sh
CHARIOX_DIAGNOSTIC_KERNEL_BINARY=/absolute/path/to/chariox-kernel \
  node --test apps/cli/scripts/lib/room-kernel-diagnostics.kernel-test.mjs
```

This test forces a local WebSocket handshake failure and removes its kernel,
listener and temporary state in cleanup. It is not a reproduction of every
private-relay provisioning failure.

One discovery/click task does not close the entire Browser matrix. Navigation,
frames, dialogs, upload/download, permissions, concurrent actors, vault-backed
work, persistence and provider-thread resume each still need their own proof.
Neither these local drills nor their reused component builds establish managed
deployment acceptance or a final packaged release.

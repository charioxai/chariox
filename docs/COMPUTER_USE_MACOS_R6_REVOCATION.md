# R6 kernel grant revocation

The M1 seat now observes shared Action/grant cancellation during blocking helper
I/O. A per-request watcher checks authority every 5 ms and closes only the paired
connection. It does not wait for the request/channel mutex. Revoke all cancels
human and agent Actions before waiting for the seat lock. Kernel shutdown uses
the same ordering. A completed watcher is joined and cannot fence a new pairing.
The helper checks authenticated socket EOF and its two-second lease at every
native fence. No TCC mutation, permission prompt or consent retry is involved.
Already completed mutations are never undone or replayed. An owned release that
cannot pass the remaining fences still reports unresolved owned input.

The existing shared Computer command DTOs are unchanged. The private bootstrap
may carry a drill scope from `CHARIOX_MACOS_COMPUTER_DRILL_SCOPE`, with exactly
`pid`, `window`, `owner_window`, and `target`. No scope keeps M1 identity mode.
A scope admits the signed sibling fixture or Apple-signed TextEdit through the
same M0 target checks. The kernel and helper retain mutual code identity pins.
Only M0 bounded AX click and short text are implemented in this mode. Physical
holds, clipboard and persistent keys refuse. Input coordinates use pixels
relative to the selected window and its bound scale, including negative Quartz
origins. Movement or scale changes end that binding and require a fresh start.
The environment setting is for disposable owner-authorized drills, not a
production scope-selection API. Agent tools remain disabled on macOS.

The native helper has not been rebuilt at this checkpoint. All r6 native changes,
including the following M2 checkpoint, will enter one separate build bundle.
The owner step is a re-grant of that final bundle. The existing granted helpers
remain intact. Live input revocation is BLOCKED until the re-grant.

Regression evidence is outside Git in
`~/.codex/evidence/browser-computer-use-macos/cumac-r6-20261010/`.
The actual Mac adapter's socket exchange and watcher were compiled in a 94 MB
isolated Rust harness with minimal host/cancellation stand-ins. Thirteen tests
passed, including real code-identity admission, socket EOF, subprocess cleanup
and in-flight cancellation. The same in-flight test without the watcher failed
after the original 20-second request timeout. This does not substitute for the
real kernel grant route. Swift channel tests passed before any native permission
or input API. Twelve M0 and two channel structural checks plus seventeen Python
runner checks passed. No running kernel, installed bundle or durable key was used.

Rollback removes the private drill scope setting and leaves identity mode. The
whole feature remains disabled and unstaged. M3 must implement physical human
observation and fatal held-input recovery; this checkpoint makes neither claim.

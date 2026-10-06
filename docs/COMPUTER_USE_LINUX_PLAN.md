# Linux Computer use: MP-08 / MP-10 / MP-11 Phase A

Phase A source: OSS main `e325afa580d81954e2c179757fc53fa02ed2a2b3`
(local435 / relay73). Work branch: `computer/unicode-input-ocr-main`.
This is an audit and proposed design, not Phase B implementation or acceptance.

## MP-08 / MP-10 / MP-11: reconcile #846

Compared #846 `0c4e848eb9997741cbde03f4c20f5de502a0794e` with the
specified main, including all seven commits since G2. There is no remaining
runtime or test patch to replay on this main. Of its 99 touched paths, 18 are
byte-identical, including the OCR implementation/tests, keyboard overlay tests,
physical fixture, image receipt tests, input action policy, hold adapter, hold
tests and session action/model changes.

Other paths retain #846's behavior with subsequent main changes: protocol
snapshots use the union versions; IPC admission was split into responsibility
modules; Computer execution gained live authorization checks; Browser artifacts
added commands/types; keyboard text rejects inherited accelerator bindings and
key chords translate Enter to X11 Return. Copying the old files would regress
these changes. The Dockerfile still installs German OCR and runs native tests.
The final #846 physical fault environment fix is already byte-identical.

Retained comparison: external `culinux/reconcile.json` and
`culinux/reconcile-current.diff`. Focused current-source checks: 131 Node tests
and 42 native helper tests pass, zero skips. These establish fixture/policy
seams, not live provider, Web/TUI or ordinary/managed acceptance. No protocol
number was allocated or changed by this lane. The coordinator can close the
obsolete #846 branch after independently checking this reconciliation.

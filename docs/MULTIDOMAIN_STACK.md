# MD-stack: Apps, host browser, App views, display and notes

The canonical integration starts at Apps queue 39633aeb4 (local 416, relay 70),
then browser a4ef64165 (417). Feature snapshots preserve416 →417 →418 →419
→424; current local version is424, relay stays70. Branches were replayed in
coordinator order: display, notes, then only the App-view commits. The App-view
replay retains424 rather than lowering it to its original418.

| MD surface | Introduced protocol | Frozen input |
| --- | --- | --- |
| MD-2/MD-3 kernel browser | 417 | Apps-based kernel browser |
| MD-APP user App views | 418 | md/integration e083f7c09, App commits only |
| MD-DISPLAY-04 transport | 419 | md/display-transport 6b288b330 |
| MD-N3 notes | 424 | md/notes 385aa5d56 |

Apps terminal/grant admission and boxed router boundaries stay authoritative.
The typed KernelBrowserDisplayRequest seam is unchanged. App views use its
existing protected browser host; generation-bound drains cannot restart a stale
host. Notes keep an independent on-demand capability and sessionless routing.
Experimental display and TUI text App flags remain off by default.

MD-3 local connection cancellation now covers dispatch, Vault and backend waits;
actor retirement reclaims presence while preserving bounded history. MD-5 cached
browser replies bind protection revisions and re-enter the capture barrier.
Mutations retain deduplication; safe reconnect reads get fresh command IDs.
Physical input errors after dispatch fence Chromium before another actor runs.

MD-4 native drills use a normal Unix user, disposable explicit CHARIOX_HOME,
sandboxed native Chromium and ordinary test stacks. Display drill executables
and Node/PyAV dependencies are supplied explicitly; no host installation or
provider account is needed. The native Mac replay still needs a Mac build/run.

Display, notes and App-view lanes should rebase new work onto this combined
stack, retaining these admission and future boundaries. Notes 979920c8 was not
part of the frozen published head; its contextual-anchor fix is a separate
follow-up. Exact replay maps, source identities, test commands/results and
cleanup belong to the lane's external PUSH_READY and evidence receipts.

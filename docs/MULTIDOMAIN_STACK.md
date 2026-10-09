# MD-stack: Apps, host browser, App views, display and notes

MP-08 / MP-10 / MP-11 current access-model amendment: reserved local **432** /
relay **78**. See [user-domain access](MULTIDOMAIN_USER_DOMAIN_ACCESS.md) for
retained grants and cross-kernel badges. The 427/74 union below is its base.

The canonical integration starts at Apps queue 39633aeb4 (local 416, relay 70),
then browser a4ef64165. The coordinator allocated local **427**, relay **74**
for the union. Unreleased feature numbers 417/418/419/424/425 are folded into
427; their shape/hash guards now pin the union version. Released snapshots
through 416/70 retain their shape history. Current feature minimums are 427.

| MD surface | Frozen input |
| --- | --- |
| MD-2/MD-3 kernel browser | Apps-based kernel browser plus priority review fixes |
| MD-APP user App views | md/integration 3e1df0081, App commits only |
| MD-DISPLAY-04 transport | md/display-transport 6b288b330 |
| MD-N2/MD-N3 notes | md/notes a7da9d748, including anchoring and quote protection fixes |
| MD-CAPTURE region screenshots | md/screenshot f1f979754 |

Room NoteObservation is snapshotted under relay 74. Notes refuse unknown or
older worker protocols before sending that variant; no legacy worker receives
an unsupported command. Visible-region capture remains an observation and
bypasses transport receipt caching so every retry enters current protection.

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
stack, retaining these admission and future boundaries. Notes review fixes are included. Cloud screenshot clients must adopt minimum
427 when binding to this union. Exact replay maps, source identities, test commands/results and
cleanup belong to the lane's external PUSH_READY and evidence receipts.

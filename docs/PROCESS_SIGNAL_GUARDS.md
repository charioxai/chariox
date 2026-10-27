# MP-11 process signal enforcement

Repository scripts, tests and drills use the shared Node guard in
`apps/kernel/slice-linux-docker/owned-process-signals.mjs` and the Python
guard in `apps/kernel/slice-linux-docker/owned_process_signals.py`.

Record an owned process when launching it (`spawnOwned` in Node;
`OwnedProcesses.record` in Python). Before every signal, the guard rejects
noninteger IDs and IDs <= 1, reads current process metadata, and verifies
the PID/start identity of every member. A live recorded session leader can
prove ownership of reparented children in its exclusive session. Without
that leader, only previously recorded identities are accepted. Unknown,
foreign, reused or unreadable identities refuse signalling.

Stream producers that can exit before their descendants settle use Node's
`retainLeader: true` option. Its small command anchor keeps the recorded
session leader alive until the command and group settle. Python's existing
command anchors use the same ownership guard. Do not fall back to an
unguarded signal after an ownership refusal.

`pnpm lint:process-signals` reports raw group sends, raw signal aliases,
Python group sends, shell group sends and name-based signals. The scanner
examines tracked and new repository source files; its regressions run in
the existing `node --test scripts/*.test.mjs` CI entry. `pnpm
test:process-signals` runs the focused scanner and both language guards.

`scripts/process-signals-allowlist.json` contains exact reviewed exceptions
with reasons and occurrence counts. Remaining name-based service operations
are restricted to isolated, selected slice containers or the legacy slice
VM; their fixed patterns cannot select the namespace init. Inert test mocks
and source assertions are separately explained. These exceptions do not
authorize host-wide name-based cleanup. Guard and scanner source exemptions
bind a full source hash. Changing an exception, adding a duplicate, removing
a site, or changing a reviewed guard requires an explicit review update.

MP-11 validation establishes these source/cleanup seams. It does not close
the ordinary/managed behavioral matrix, independent current security review,
signed release or fresh-machine acceptance gates.

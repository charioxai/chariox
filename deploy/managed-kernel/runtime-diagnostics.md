# MP-07/MP-08/MP-10/MP-11 A→B diagnostic campaign

These optional, payload-free journals observe the ordinary kernel; they do not
control prompt state, relay admission, release settlement or shutdown. Enabling
observation changes no serialized client protocol (local 472 / relay 73).

MP-11: the only record fields are schema, timestamp, PID and an enumerated event.
No prompt, transcript, provider payload, credential, token, error text, URL or
config file enters the journal. Files are private, fsynced at each event and
bounded to 8 MiB per process. The observer accepts only the same strict schema,
with a 10,000-record campaign quota. Its content hashes and timestamps are
supplementary evidence; this public endpoint is not guest authentication.

MP-07/MP-10: prepare `/home/chariox/.chariox/runtime-diagnostics` with owner
`chariox:chariox`, mode 0700. In the campaign image, enable
`CHARIOX_RUNTIME_DIAGNOSTICS_DIR=/home/chariox/.chariox/runtime-diagnostics`
through the signed `enable-campaign-diagnostics.sh <exact-HTTPS-observer-url>`
campaign-image installer after signed image installation and before enrollment.
It refuses a running guest or existing campaign configuration and enables a
separate, bounded shipping unit on first boot. MP-07/MP-11: the signed upgrade
policy admits only its exact bootstrap drop-in path and bytes: the single
diagnostics-directory assignment. The file must be root-owned, mode 0644,
single-linked and regular, under root-owned directories without symlinks or
group/world write permission. Additional, changed or worker-service drop-ins
remain rejected, as does an unreloaded systemd configuration. The
supervisor inherits it into the kernel; the kernel passes it as a literal
systemd argument to its detached upgrade unit. Ordinary kernels may enable the
same variable. With no variable, the kernel does no diagnostic I/O. Upgrade
phase recording is best effort and cannot change a successful release result.
An absent or invalid directory produces no journal: require positive guest and
observer records before beginning the A→B cell.

MP-07/MP-08/MP-10: records distinguish user prompt dispatch from provider dispatch
start, return and failure; provider dispatch return does not prove provider turn
completion. Heartbeat success means a real relay socket write, not relay receipt
or Cloud freshness. Separate Cloud presence start/acknowledged/failed records
wrap the real signed presence HTTP request; they never contain its body. Upgrade polling, download, unit launch, recovery, pending
settlement, applied/failed evidence and Cloud response are recorded. The detached
transaction records prepared before stopping the kernel, then stopped, activated,
committed or rolled_back after the corresponding durable phase write. Download
records bracket the complete download, not byte progress. Bind PID/timeline to
the exact signed release, environment and machine in the campaign's independent
product receipts; run one transition at a time.

MP-07/MP-10: the script is included in the signed source build context at
`/usr/lib/chariox/slice-build-context/deploy/managed-kernel/runtime-diagnostics.py`.
Use its `receive` mode on b3 with an evidence directory (0700), a lane-owned
loopback port and an exact `/path1-diagnostics/<campaign>` path. Route only that
path through the owned PR stack's HTTPS ingress using the reviewed stack tooling.
Never edit shared Caddy manually. Keep the listener alive through guest teardown.
The guest's independent systemd shipping unit runs `ship --directory <guest-dir>
--observer-url <exact-HTTPS-path>` every five seconds, outside the bootstrap and
update cgroups. It uses standard TLS validation, rejects redirects and URLs with
credentials, and never changes shared provider logins. Retries are idempotent.
Observer acknowledgement follows file and directory fsync. A failed request
retains the guest journal and retries; no acknowledgement is manufactured.

MP-07/MP-09/MP-10: before teardown, capture the failing real client screenshot and
product state, run `ship --once` through the real guest provider shell while
that shell is still alive, and save its ACKNOWLEDGED_SNAPSHOT receipt on b3.
The count is distinct canonical records, not event multiplicity; identical
records are replay-equivalent. The snapshot hashes the sorted distinct record
SHA256s, so b3 can recompute it from the corresponding receipt filenames. Compare
its count and snapshot hash against those observer records. This proves the collected cutoff, not records written later.
Keep continuous shipping alive until actual provider deletion. For a final
snapshot with producers stopped, arm a detached validation unit through the
real provider first; that unit must stop bootstrap/update/ship, run `ship --once`
and retain its public receipt independently of the killed provider. Do not
expect a provider inside the stopped bootstrap cgroup to collect afterward.
Collection failure is RED, never a diagnostic pass. The independently armed cost/cleanup watchdog retains its authority and
must still delete resources at its deadline; diagnostics cannot postpone that
bound. Continuous shipping keeps earlier evidence on b3 if the guest becomes
unreachable. Record an incomplete final snapshot honestly before forced deletion.
Stop the owned listener and PR stack at round end; retain evidence, remove only
owned guest runtime state and exact resources. No paid campaign is authorized by
this document.

MP-07/MP-08/MP-10/MP-11: focused checks cover durable append, private paths, size
bounds, strict schema, symlink/hardlink rejection, truncated records, exact
acknowledgements and replay. These are supplementary checks. Real hosted A→B,
provider and heartbeat diagnostic coverage remains for the signed paid rerun;
no MP item closes from unsigned inputs or source tests.

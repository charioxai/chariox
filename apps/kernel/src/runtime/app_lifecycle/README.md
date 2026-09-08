# Retained App worker lifecycle

`AppControlService` retains this service once per kernel. The existing transport
runtime pump calls its bounded recovery scan after the kernel publication gate.
Kernel shutdown joins its blocking owners before the remaining daemon cleanup.
No App view or terminal connection is needed for an approved backend to recover.

This slice starts only an **already approved active installation generation**.
The writer mints an internal start admission from the current installation and
enrolled publisher. Startup reopens the held stored archive, verifies its exact
digest and signature, verifies the sealed release tree, and compiles the catalog
from those verified bytes. Linux then uses the enrolled runtime and the existing
production `PreparedWorker::prepare_linux` factory. Native readiness, SDK handler
registration and the writer's current activation confirmation must all succeed
before the weak callable handle becomes visible to MCP discovery or event pumps.

There are four aggregate live/preparing worker slots, one package/preparation
slot, and the existing eight shared App operation slots. Start and stop actions
serialize per owner/installation. A replacement cannot acquire the old owner's
slot until its process is reaped and its single SDK peer's broker work drains.
Every thread retains its resource and artifact leases through that cleanup, even
if a requesting future is cancelled. Worker threads retain a weak-handle
publisher, not the service that joins them, avoiding an ownership cycle.

The same SQLite writer persists one health/restart-intent row per installation.
Generation and attempt fences prevent old cleanup from overwriting a replacement.
Current installation and publisher admission are rechecked before preparation,
activation, Running publication, and periodically while the owner lives. The
periodic check retains one monotonic budget through shared-admission contention.
Each ordinary SDK call continues to use its own current catalog/permission fence.

A stop request withdraws tool/pump handles immediately, including when App
operation slots or SQLite are busy. Draining permits the existing state/events
and atomic-file flush methods; new HTTP or asset requests are rejected. The
shutdown callback has a three-second bound, after which the common peer/process
supervision finishes cleanup. `Busy` from a saturated stop means durable/join
confirmation is pending; it does not restore a withdrawn worker. A completed
stop returns only after the actual owner is joined and the stop row commits.

A user stop persists `desired_running = false`. Graceful kernel shutdown retains
the previous restart intent. Restart recovery scans eight installations per
pass with a rotating cursor and a five-second minimum scan interval. A failed
generation requires an explicit internal restart; this slice does not silently
retry revoked or repeatedly failing code. Runtime health rows are observations
and restart intent, never an installation approval or sandbox attestation.

## Verification and remaining integration

Focused tests use the real durable writer and the fixed libc worker fixture:
headless recovery, duplicate starts, saturated stop with a held SQLite write
lock, actual reap/lease release, concurrent shutdown joins, restart after reopen,
publisher revocation, and preparation failure. The native fixture never executes
App payload or Node; these tests therefore establish ownership and writer
integration only. Separate archive tests check anchored reopen, shared lease,
digest mismatch, oversized input and symlink rejection. Hosted execution remains
required before these new tests are recorded as passing.

The next control slice must connect the existing verified-upload preparation and
human/policy decision to first-install readiness and atomic commit. It must then
use this same retained lifecycle owner, rather than constructing a second owner.
No new terminal request or protocol version is introduced here; start/stop/status
are currently internal kernel APIs. Production macOS preparation/signing,
end-to-end signed embedded Node execution, heartbeat/quarantine and bounded
restart policy, user-visible log/status projection, and update data migration
remain explicit Phase 1 work. Physical Linux provisioning and its hosted tests
are separate evidence from these fixed-worker lifecycle tests.

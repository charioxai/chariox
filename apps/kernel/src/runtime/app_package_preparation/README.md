# Verified package preparation

`AppPackagePreparation` is created once by the kernel's App service and shares
its existing upload service and durable database. Its clones share one preparation
semaphore. The caller supplies an already authenticated owner and an opaque
completed upload handle; no path, publisher key, clock, approval or verification
boolean comes from a terminal request.

The shared AppControl permit and single preparation permit are acquired before
any finalized archive is read or allocated. The blocking task owns both until it
finishes, even when its awaiting caller disconnects. Finalization rechecks the
archive digest and retains the existing upload descriptor lease while the service
reads at most 128 MiB plus one size-check byte. Package identity only selects an
owner-scoped enrolled key; the exact signature and all package content still pass
the offline package verifier. The resulting immutable candidate retains that
successful key's enrollment revision.

Release publication creates a fixed private leaf derived from the kernel's
existing database filename. Descriptor-relative creation checks the existing
parent and syncs the child and parent. Publication retains its anchored directory
through return; a caller cannot turn a display path into verification evidence.
The preparation object holds the owner, immutable candidate and anchored release
together. Access to the candidate/directory checks the supplied trusted owner.

The operation has a 512 MiB reservation ceiling, uses ReleaseStore's conservative
allocation-unit accounting, measures free disk and leaves a 4 GiB host reserve.
Only one preparation writes through this kernel service at a time. These are
preparation limits, **not the full aggregate App storage quota**: accounting for
active data, all retained releases/snapshots, logs and competing kernel storage
still needs the shared storage allocator. No unchecked quota claim is accepted
from a client.

A successful preparation does not create an installation, approve capabilities,
mark worker preparation complete, execute migrations or launch code. The next
installer step passes its candidate through the verified durable writer, which
rechecks the exact enrollment revision atomically. Revocation during or after
publication can leave an unused immutable release, but cannot authorize staging
or activation. Published bytes confer no permission.

The original finalized upload retains its original expiry. Retrying before that
expiry re-verifies the package and reuses an exact published digest. A publication
I/O failure returns `PublicationInterrupted`: publication may be visible, so the
retry uses ReleaseStore's existing tree validation, interrupted sealing repair and
parent-directory sync before reporting success. Recognized abandoned temporary
stages are collected before publication; final published releases are retained.
After upload expiry, sending the same package again permits the same safe reuse.
No second durable receipt or installation authority is introduced.

Focused tests cover exact publication, owner isolation, incomplete uploads,
unenrolled and substituted keys, no installation residue, descriptor identity,
service reopen/reuse, lost publication reply, revocation before writer staging,
and admission retained across await cancellation. Runtime containment, signed
worker execution, whole-host storage quotas and authenticated consent remain
separate release gates.

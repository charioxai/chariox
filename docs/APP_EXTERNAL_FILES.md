# MP-08 / MP-09 / MP-10 / MP-11 — App file grants

## File gestures (FILES-OPEN-01)

A trusted client drop or open-with gesture is the selection. It must enter the
same one-file grant/import path as a picker answer, without a second picker.
The client transmits only the selected basename and bytes over its authenticated
kernel connection. An App never sends an original pathname or a claimed user
gesture to authorize host access. Agent artifact references stay a distinct
entry point and do not authorize reading arbitrary host files.

Admission checks the active signed manifest's `user_selected` declaration,
installation identity, filename/type and byte limits. The kernel binds the
handoff to that installation, persists the grant and supplies an opaque ID to
its backend. Import copies into private data once; export remains a separate
owner offer. Remote clients send a copy to the kernel's machine. The client
must bind its selected App and outstanding gesture before upload and reject
stale/replaced views. Current client-host App cutover and desktop handoff still
need to carry these gestures; existing picker acceptance alone is insufficient.

An import that may have published must retain its consumed grant. A lost
publication reply cannot make the grant reusable or report that no effect
occurred. Only a definitely pre-publication failure may release the claim.
A successful rename followed by an uncertain sync is an uncertain effect, not
permission to replay. This uses the existing private-file publication boundary.

## Permanent folders (FILES-FOLDER-01, design before implementation)

Add a signed `user_folder` external-file capability. Installation approval
shows this files permission and a permission-changing update requires approval.
The backend asks through `host.pick_folder`; the trusted kernel prompt lets the
owner select a folder **on the kernel's machine** and choose read-only or
read-write. A remote browser's local upload chooser is not a folder grant.
An agent or App cannot answer this prompt. A missing grant fails explicitly.

Persist a random opaque grant ID, owner, installation identity, selected
absolute kernel-local path, filesystem device/inode identity and access mode.
The path is private kernel state and never part of an SDK result, event or
trace. A worker gets no additional preopen, mount or host path. Settings list
and revoke grants; uninstall removes them. A replacement installation cannot
inherit a deleted grant. An update removing the capability ends access; adding
it again requires a fresh choice. The owner may revoke a grant while the App
is running.

Every list/read/write goes through the kernel broker, with the actual worker's
owner, installation and generation supplied by the runtime. An agent's discovery
grant does not substitute for the App's folder permission. Check active signer,
release, capability and grant immediately at operation admission and publication.
Limit operation concurrency/rate, file bytes, listing entries and cumulative
write quota. Rejections report the bound and how to recover.

Resolve relative components from held directory descriptors, refusing absolute
paths, empty components, dot-dot, symlinks, device files, sockets and writable
hard-link aliases. Open directory components with no-follow semantics. Verify
root device/inode identity on every call; a missing, moved or replaced selected
folder ends access until a fresh selection. A path check followed by an ordinary
open is insufficient: the operation must use the checked descriptor. Retain
parent descriptors through final rename and verify the selected root still
contains the target at publication. A directory moved out between check and use
must fail, not write through an old descriptor into a now-unselected tree.

Revocation and publication have a single ordered boundary: a write that has
not crossed publication is cancelled; one already published returns its actual
receipt or an explicit uncertain outcome. Stage/sync file bytes outside the
state writer, then fence the grant and current installation at publication.
Do not claim filesystem and SQLite commit atomicity. No automatic replay after
an uncertain rename or acknowledgement.

Before acceptance, real-entry regressions must fail first for traversal,
symlink swaps, ancestor rename races, root disappearance/replacement, revoke
while waiting/in flight, read-only writes, quota/rate/concurrency limits,
agent calls without an App grant, uninstall/reinstall and permission-changing
updates. Notes-over-Markdown and photo-directory fixtures must run with real
contained workers on Linux and macOS, including remote-kernel clients. Mac
validation needs the actual Mac; a Linux implementation or test cannot stand
in for it. No permanent-folder implementation or acceptance is claimed here.

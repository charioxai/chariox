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

## Permanent folders (FILES-FOLDER-01)

Add a signed `user_folder` external-file capability. Installation approval
shows this files permission and a permission-changing update requires approval.
The backend asks through `host.pick_folder`; the trusted kernel prompt lets the
owner select a folder **on the kernel's machine** and choose read-only or
read-write. A remote browser's local upload chooser is not a folder grant.
An agent or App cannot answer this prompt. A missing grant fails explicitly.

Persist a random opaque grant ID, owner, installation identity, selected
absolute kernel-local path, filesystem device/inode plus stable creation/generation identity and access mode.
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
parent descriptors through final rename. Admission authorizes the selected
filesystem objects: a rename after admission cannot redirect that operation to
a replacement pathname. An admitted operation may finish against its pinned
object; subsequent calls resolve from the selected root again. Tests must
separately assert that a moved root is refused on the next call and that an
ancestor or symlink swap cannot redirect an admitted operation. Do not promise
to undo an already admitted filesystem operation when another process renames
its target. Across restart, refuse a root whose stable identity cannot be
proved, rather than treating a reused inode number as the selected directory.

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
in for it. Acceptance is recorded per platform and client in the external drill ledger;
implementation on Linux does not establish the macOS and remote-client cells.

### SDK and owner controls

The signed declaration is `external_files: ["user_folder"]`. `host.pick_folder`
(`host.pickFolder()` in JavaScript) waits for a trusted owner choice and returns
an opaque grant. `folders.list(grant, relative)`, `folders.read(grant, relative)`
and `folders.write(grant, relative, bytes)` carry relative paths and bounded
bytes through the kernel broker. They give the worker no external preopen.
Listings contain safe regular files and directories only; unsupported entries
are omitted. Names reserved for unpublished writes are neither listed nor
readable. A contained App can use Markdown and photo bytes through the same API.

A grant permits 60 calls per minute, at most 512 KiB per file operation, at most
1,024 entries per listing, and 512 MiB cumulative admitted writes. The write
budget is spent before staging and is retained when an operation is cancelled,
fails or has an uncertain outcome. The owner can revoke an unused grant and
make a fresh selection when the write allowance is exhausted. Each installation
may retain 32 grants and four pending choices; a choice expires after ten minutes.

Web chooses on the request's kernel using its directory browser. The shared
text-terminal commands are `/app folder grant OP "KERNEL-PATH" read-only`
(or `read-write`), `/app folder list INSTALLATION` and
`/app folder revoke INSTALLATION GRANT`. Paths never refer to the terminal
client's local filesystem. Extensions exposes the same owner's list and revoke
controls. Changing the signed permissions or uninstalling removes grants.
`FOLDER_CHANGED` requires a fresh selection. `READ_ONLY`, `RATE_LIMITED` and
`QUOTA_EXCEEDED` name the refused bound. `APP_FOLDER_OUTCOME_UNCERTAIN` means
inspect the destination before doing anything further; it never authorizes replay.

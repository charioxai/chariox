# Private macOS App storage

This private production module prepares and recovers the writable filesystem
roots needed by `PreparedWorker`. Its methods are blocking and intended for the
worker ownership thread. The signed runtime factory has not been connected yet;
this module does not authorize an App or launch one.

`StorageRoot::open` accepts an existing kernel-owned private directory. The
kernel supplies the owner, installation, and generation to `prepare`; none is an
App argument. A returned `MountedStorage` retains the installation lock, backing
file identities, and mounted directory descriptors. The worker supervisor must
retain it until the worker has been reaped and every broker request borrowing a
directory descriptor has finished. Before preparing a later generation or
calling `recover_all_blocking`, the kernel must quiesce those previous workers
and requests. Storage recovery cannot decide whether a worker is still running.

The candidate policy provides a 512 MiB data image and a separate 64 MiB temporary
image per installation. Creation uses Apple's explicit `-megabytes` option with
the validated integer 64 or 512; the `-size` suffix `b` means 512-byte sectors and
must never be used as a byte suffix. Data survives generations; temporary storage is replaced
only after the previous image has been identified and detached. Both images are
fixed UDRW files containing APFS, with no sparse-image or automatic growth mode.
Filesystem metadata reduces usable capacity. The journal reserves another 2 MiB
per image for its UDIF envelope; an observed larger image is rejected. Admission
allows at most 64 installation directories, 32 GiB of total recorded reservation,
and requires 8 GiB of current host free space after all unallocated reservations.
These are internal candidate limits, not a public quota API. File length does not
prove physical preallocation: admission measures allocated blocks and reserves
the remainder. Other host activity can still consume host space.

Each installation lives at `installation-<sha256(owner + NUL + installation)>`.
Its mode-0700 directory contains `storage.json`, random per-image filenames, and
the fixed empty `data` and `tmp` mountpoints. The journal is bounded, written with
the existing descriptor-based atomic replacement/fsync helper, and records
creation intent before a tool can create an image. File device/inode identities,
the mountpoint identities, and discovered APFS UUIDs become durable before a
worker can receive the roots. Paths and their ancestors must remain controlled
by the kernel installer; the App receives access to the mounted roots only.

Every attach uses the fixed trusted `/usr/bin/hdiutil` with `-nomount`; mounting
then uses `/usr/sbin/diskutil` with `noexec,nodev,nosuid,owners` at the fixed private
mountpoint. `fstatfs` on the held directory must confirm those flags, APFS, the
exact BSD volume device, and the exact mountpoint. Fresh hdiutil image-path
metadata and diskutil device/name/UUID metadata authorize every detach. No device
number is reused from an earlier journal, and cleanup never force-detaches a
volume. The current discovery expects hdiutil to list the physical image device
before a synthesized APFS container; only the hosted drill can confirm this on
the supported macOS image.

Apple tools use the same bounded, clean-environment `posix_spawn` primitive as
the worker. Arguments are fixed and bounded; stdout is limited to 1 MiB and
discarded diagnostics to 64 KiB. A preparation has a 120-second command budget,
cleanup 60 seconds, individual Apple commands at most 90 seconds, and plist
conversion at most 5 seconds. Every owned child is terminated and reaped on
failure or timeout. The installation lock is inherited as FD3 by these **trusted
Apple tools** and remains held while they retain that descriptor. Whether Apple
closes it or passes it to a longer-lived helper requires hosted observation;
inheritance alone does not prove ownership across an in-flight kernel crash.
This does not change the App worker ABI, where
FD3 remains the SDK channel. Inheritance by longer-lived DiskImages helpers is
an explicit hosted recovery check; such helpers can also run outside the sampled
build process group.

`release_blocking` records pending recovery before detaching. It closes its own
mounted descriptors, verifies exact current image/device identities, detaches,
and proves the original empty mountpoints are visible again. Only then does it
clear the pending marker. Drop performs the same synchronous bounded recovery
unless an explicit cleanup was already attempted. A failed explicit release does
not silently acquire a second cleanup deadline. If cleanup fails, the journal
and capacity remain reserved for the next owner. There
is no detached cleanup task, image deletion on failure, or reported capacity
reclamation. A crash during first creation can discard only the uncommitted
image named by that private creation intent. Existing data with a committed UUID
is never silently recreated when missing.

Eight ordinary tests exercise parsing, identity checks, journal fsync recovery,
interrupted temporary files, fixed command arguments, capacity accounting, and
preserved recovery after an explicit cleanup attempt.
They never call hdiutil or create a filesystem. The dedicated
`app-storage-macos.yml` workflow executes ignored tests against this same module
on a disposable GitHub macOS runner, using 64 MiB images for both roots. Its
required observations are actual mount flags and ownership, denied execution,
ENOSPC/EDQUOT at each filesystem boundary, preserved data and UUID across a normal
restart, discarded temporary files, and recovery after a separate process exits
without running Drop after its tool commands completed. It does not yet prove
recovery while hdiutil or a DiskImages service is processing an operation. A
final production recovery call must succeed before the
wrapper deletes its images. Failed recovery retains the images and journals
until disposal of that dedicated runner. The wrapper records bounded logs and
metadata; its process/resource watch is monitoring, not an OS hard limit.

The first hosted run, [34176513133](https://github.com/charioxai/chariox/actions/runs/34176513133),
caught the original size-unit error: `67108864b` requested 32 GiB instead of
64 MiB. The post-create size check rejected that oversized image before any App
ran; its recovery journal was retained on the disposable runner.
Run [34177429718](https://github.com/charioxai/chariox/actions/runs/34177429718)
confirmed a correctly sized image, APFS UUID, required mount flags, exact device
and mountpoint, and successful cleanup. Its remaining rejection was the new
APFS root's 0755 mode despite `hdiutil -mode0700`. Preparation now applies 0700
and fsync to the verified same-owner mounted descriptor before worker admission.
Run [34177801710](https://github.com/charioxai/chariox/actions/runs/34177801710)
then passed the complete dedicated storage drill at commit `27c2472d3`: each
image occupied exactly 67,108,864 bytes and exhausted writable space after
63,963,136 bytes. Required mount flags and denied execution, persistent data and
UUID, temporary-data reset, process-exit recovery, and final cleanup passed.
The real storage test completed in 22.10 seconds; eight offline tests also passed.
This is also why a post-create length check is not a hard precreation bound.
An inherited process file-size limit would cover only writers that actually
inherit it; DiskImages service ownership must be observed before asserting that
such a limit constrains image creation.

In-flight tool/service
ownership across a kernel crash also needs validation before factory integration;
if FD inheritance cannot provide it, a trusted supervised guardian is required.
This component does
not yet establish worker integration, signed/hardened library validation,
snapshot and rollback semantics, an installation-wide resource admission
service, or a host disk reservation immune to unrelated writes.

The command contract follows the installed Apple `hdiutil(1)` and `diskutil(8)`
manuals, including plist output, fixed UDIF creation, separate attach/mount,
mount options, and identity-based detach. Apple also describes fixed read/write
images in [Disk Utility Help](https://support.apple.com/guide/disk-utility/create-a-disk-image-dskutl11888/mac)
and the APFS tool model in its [archived APFS guide](https://developer.apple.com/library/archive/documentation/FileManagement/Conceptual/APFS_Guide/ToolsandAPIs/ToolsandAPIs.html).
The hosted tool output is the compatibility gate for the current macOS release.

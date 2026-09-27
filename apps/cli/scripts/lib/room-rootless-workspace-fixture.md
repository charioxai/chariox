# Room rootless fixture workspace

Provider-free Room pointer/click drills have two explicit workspace modes:

- With `CHARIOX_SLICE_DOCKER_BROKER_SOCKET`, the drill requests the kernel's existing `empty` slice development setup. The kernel owns publication, managed rootless ACLs, and revisioned `DeleteSlice` cleanup.
- Without a broker, the operator must set `CHARIOX_ROOM_DRILL_FIXTURE_WORKSPACE_ROOT` to a canonical engine-visible temporary root such as `/var/tmp`. Before creating anything, the drill verifies that `chariox-docker` can traverse that root. It creates one unique empty child, changes permissions only on that child, then verifies the engine can traverse and write it. Repository and home roots, their descendants, and their ancestors are rejected.

Direct-mode cleanup runs only after the kernel, TUIs, container, and other producers stop. It checks the recorded device, inode, owner, direct parent, and generated prefix before deleting the child, refuses a replaced path, and verifies zero residue. It never changes the selected root or its ancestors. Broker-mode cleanup continues to validate the exact workspace recorded in the returned publication and verifies its slice-owned storage root is gone.

Real-provider mode keeps its existing private `mkdtemp` workspace behavior. Kernel state, credentials, logs, and retained evidence remain in their private roots and are never written to the rootless publication.

# Room rootless fixture workspace

The provider-free Room pointer/click drill requests the kernel's existing `empty` slice development setup. The kernel creates the empty workspace beneath its configured slice development root, publishes its ownership receipt atomically, and applies the managed rootless broker ACL contract. On a managed Linux host that root is `/var/lib/chariox-slice-share/slices/development`.

The drill accepts only the exact workspace recorded in the returned empty-development publication. It must be outside the source repository, directly below storage named for the slice, and the publication may own no other repository path. `DeleteSlice` remains the sole cleanup authority; after producers stop, the drill verifies that both the workspace and its slice-owned storage root have no residue.

Real-provider mode keeps its existing private `mkdtemp` workspace behavior. Kernel state, credentials, logs, and retained evidence remain in their private roots and are never written to the rootless publication.

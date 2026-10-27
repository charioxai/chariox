# Rootless browser-state drill workspace

The provider-free persistence drill requires one explicit workspace mode. With `CHARIOX_SLICE_DOCKER_BROKER_SOCKET`, it uses the kernel-owned empty-development publication. Without a broker, set `M20_WORKSPACE_ROOT` to a canonical engine-visible temporary root such as `/var/tmp`; the drill probes traversal as `chariox-docker`, creates one inode-tracked empty child, and changes permissions only on that child.

The repository, home, runtime state, credentials, and evidence remain outside the mounted fixture. Cleanup runs after the slice, container, fixture server, kernel, and other child producers stop. Direct mode refuses replacement paths before removal; broker mode verifies the kernel-owned publication has no residue.

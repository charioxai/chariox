# Remote dummy AEGS staging

This package runs the public dummy AEGS on a dedicated AEGS staging host. It
does not weaken the production AEGS packages: the image is pinned to its exact
local image ID, the service binds only to loopback, Caddy owns public TLS, and
systemd restores the service after a host reboot.

Create `/etc/chariox/event-publication/host-role` with exact contents `aegs`.
Create `/etc/chariox/aegs-dummy` as a root-owned mode-`0700` directory. Store
the producer and management tokens inside it as UID/GID `10001:10001`, mode
`0400`, non-symlink files. The root-only parent prevents host users from
traversing to the files while the ownership lets the non-root container read
the file-backed Compose secrets on Linux. Compose cannot remap ownership for a
file-backed secret. Keep the separate `/etc/chariox/aegs-dummy.env` root-owned
and mode `0600`:

```sh
CHARIOX_AEGS_DUMMY_IMAGE=sha256:<exact-local-image-id>
CHARIOX_AEDS_EVENTS_URL=https://<aeds-host>/v1/events
CHARIOX_AEGS_PRODUCER_TOKEN_FILE=/etc/chariox/aegs-dummy/producer-token
CHARIOX_AEGS_MANAGEMENT_TOKEN_FILE=/etc/chariox/aegs-dummy/management-token
```

Install this directory at `/opt/chariox-aegs-dummy-staging`, install the unit at
`/etc/systemd/system/chariox-aegs-dummy.service`, and configure Caddy from the
example. Only the dummy AEGS should be active while running remote staging
acceptance.

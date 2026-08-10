# Remote dummy AEGS staging

This package runs the public dummy AEGS on a dedicated AEGS staging host. It
does not weaken the production AEGS packages: the image is pinned to its exact
local image ID, the service binds only to loopback, Caddy owns public TLS, and
systemd restores the service after a host reboot.

Create `/etc/arroba/event-publication/host-role` with exact contents `aegs`.
Create `/etc/arroba/aegs-dummy` as a root-owned mode-`0700` directory. Store
the producer and management tokens inside it as UID/GID `10001:10001`, mode
`0400`, non-symlink files. The root-only parent prevents host users from
traversing to the files while the ownership lets the non-root container read
the file-backed Compose secrets on Linux. Compose cannot remap ownership for a
file-backed secret. Keep the separate `/etc/arroba/aegs-dummy.env` root-owned
and mode `0600`:

```sh
ARROBA_AEGS_DUMMY_IMAGE=sha256:<exact-local-image-id>
ARROBA_AEDS_EVENTS_URL=https://<aeds-host>/v1/events
ARROBA_AEGS_PRODUCER_TOKEN_FILE=/etc/arroba/aegs-dummy/producer-token
ARROBA_AEGS_MANAGEMENT_TOKEN_FILE=/etc/arroba/aegs-dummy/management-token
```

Install this directory at `/opt/arroba-aegs-dummy-staging`, install the unit at
`/etc/systemd/system/arroba-aegs-dummy.service`, and configure Caddy from the
example. Only the dummy AEGS should be active while running remote staging
acceptance.

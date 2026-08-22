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

## Backup and restore

Backups stop only the dummy workload, copy the SQLite database and optional WAL,
write a SHA-256 sidecar, and restart the workload. The default backup directory
is `/opt/chariox-aegs-dummy-staging/backups`. Run the scripts as root so the
directory is root-owned and mode `0700`; database artifacts are mode `0600`.

```sh
sudo /opt/chariox-aegs-dummy-staging/backup.sh
sudo /opt/chariox-aegs-dummy-staging/restore.sh --yes \
  dummy-YYYYMMDDTHHMMSSZ-PID-SEQUENCE.db
```

Restore accepts only a filename from that backup directory and verifies its
exact DB/WAL manifest before stopping the workload. Restore stages incoming
files before replacing live state. It keeps the exact prior DB, WAL, and SHM
until the restored workload passes Compose `start --wait`. A failed swap or
health check restores that prior state and runs the same health gate again. If
rollback or restart cannot be verified, restore does not start either state and
requires manual recovery. Restore uses one bounded Compose `kill` fallback when
a normal stop fails and verifies the exact service is no longer running. If
Docker is unavailable, restore retains its checkpoints and reports the runtime
state as unknown. Both scripts require a dedicated, non-symlink, owner-only backup
directory, the `aegs` host-role marker, and exactly one Compose-owned
`aegs-data` volume. Backup names are collision-resistant and are never reused.
Neither command deletes containers or volumes.

Backup and restore share `/run/lock/chariox-aegs-dummy-backup-restore.lock`.
They create it atomically, record the owning PID and operation, and remove only
a lock they own. A concurrent operation fails without contacting Docker. The
scripts verify the exact Compose project and service are stopped before reading
or replacing SQLite files. A successful stop that leaves the container running
therefore triggers the same bounded kill and verification path.

During the one-time project-name cutover, back up the retained old project
before stopping its systemd unit:

```sh
sudo env CHARIOX_AEGS_COMPOSE_PROJECT_NAME=arroba-aegs-dummy-staging \
  /opt/chariox-aegs-dummy-staging/backup.sh
```

After the `chariox-aegs-dummy-staging` project has created its new data volume,
copy the database file, its optional `-wal` file, and the `.sha256` manifest into
the new release's `backups` directory. Run the normal restore command only after
all copied artifacts match the manifest. Keep the old volume until the rollback
window closes. Never pass `--volumes` to Compose and never run a global Docker
prune on the AEGS host.

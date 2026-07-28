# Event publication deployment

This directory is a cross-repository local rehearsal while the dedicated
Hetzner hosts are unavailable. It expects `arroba-aeds` and each
`arroba-aegs-<provider>` checkout beside this OSS checkout. The services use
separate identities, secrets, networks, limits, and durable data volumes. Host
ports bind to loopback.

Each production AEGS builds from its independently owned private repository.
The public dummy AEGS builds from this repository. Identities, credentials,
stores, limits, and webhook ports remain isolated per instance.

Run:

```sh
./prepare-secrets.sh
docker compose up --build -d --wait
./drill.sh
```

`prepare-secrets.sh` also pins the selected loopback host ports in the ignored
compose `.env` file. Set `ARROBA_AEDS_KERNEL_PORT`,
`ARROBA_AEDS_PRODUCER_PORT`, `ARROBA_DUMMY_AEGS_PORT`,
`ARROBA_GITHUB_AEGS_PORT`, `ARROBA_JIRA_AEGS_PORT`,
`ARROBA_LINEAR_AEGS_PORT`, `ARROBA_GITLAB_AEGS_PORT`,
`ARROBA_SENTRY_AEGS_PORT`, or `ARROBA_SLACK_AEGS_PORT` before running it when
the defaults are occupied.
Subsequent compose rebuilds and restarts continue using those same ports.

Configure a local kernel after replacing `<stable-kernel-id>` in
`secrets/aeds-kernel-tokens.json` with that kernel's persisted daemon identity:

```sh
export ARROBA_AEDS_URL=ws://127.0.0.1:43130
export ARROBA_AEDS_TOKEN="$(<secrets/local-kernel-token)"
export ARROBA_EVENT_ENVIRONMENT_ID=local-container
export ARROBA_AEGS_MANAGEMENT_TARGETS_FILE="$PWD/secrets/kernel-aegs-management-targets.json"
```

The synthetic deployment drill uses `local-container-kernel`; a real kernel always
uses its stable persisted identity and AEDS must authorize that exact identity.

Use `docker compose down` to stop the rehearsal. Do not add `--volumes` when
testing restart or upgrade continuity. Use it only for explicit final cleanup.
The files below `secrets/` and `backups/` are ignored.

The fallback runs dummy, GitHub, Jira Cloud, Linear, GitLab, Sentry, and Slack
AEGS instances with separate tokens, webhook secrets, stores, volumes, and
loopback ports. Provider instances remain in webhook-only mode until their
public base URL, encrypted credential key, and provider application credentials
are configured.

This proves packaging, isolation, durable restart, and local product transport.
The drill also exercises capability-protected AEGS authorization and provider
resource enumeration before creating and delivering the dummy event route.
It does not replace the two-host Hetzner TLS, reboot, backup/restore, or
cross-machine failure gates in the implementation plan.

Create a consistent AEDS backup through the private AEDS repository:

```sh
../../../arroba-aeds/deploy/backup.sh
```

Restore an owned backup after checking its SHA-256 sidecar:

```sh
../../../arroba-aeds/deploy/restore.sh --yes aeds-YYYYMMDDTHHMMSSZ.db
```

Both scripts resolve the Compose-owned AEDS volume by labels and refuse an
ambiguous target. A restore never accepts a path outside the private AEDS
repository's ignored `backups/` folder. Because AEDS uses SQLite WAL mode, a backup may include
a checksum-protected `<name>.db-wal` sidecar; the database and WAL are one backup
set and must be retained together. The AEGS subscription stores use the same
stop/copy/checksum pattern when promoted to persistent hosts.

`host-separation-preflight.mjs` is the only production-facing orchestration
kept here: it performs a read-only check that AEDS, AEGS, and the existing
relay resolve to distinct machines. All service-specific containers, TLS
examples, systemd units, migrations, and operational scripts belong to their
private component repositories.

Every production AEGS also exposes a capability-protected, authoritative
`PUT /v1/subscriptions/reconcile` endpoint. It persists logical event interests
separately from AEDS routes, so provider webhooks can be normalized to the correct
interest key without giving AEDS provider credentials or source-specific state.

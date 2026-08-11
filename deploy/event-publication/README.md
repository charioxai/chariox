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
./drill.sh
```

For a resource-safe rerun of only the AEDS + dummy AEGS vertical slice, including
durable delivery, restart, and backup/restore, run `./drill.sh --core-only`.
This mode does not replace the default first-wave provider matrix.

The drill limits Compose to one build at a time. It keeps AEDS running and
builds, starts, exercises, stops, and removes exactly one AEGS at a time:
dummy, GitHub, Jira Cloud, Linear, GitLab, Sentry, then Slack. Peak runtime is
therefore AEDS plus one AEGS. Every AEGS has an explicit Compose profile, so a
plain `docker compose up` starts only AEDS and cannot accidentally launch the
whole integration matrix.

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

The fallback validates dummy, GitHub, Jira Cloud, Linear, GitLab, Sentry, and
Slack sequentially with separate tokens, webhook secrets, stores, volumes, and
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

`host-separation-preflight.mjs` performs the mandatory read-only check that
AEDS, AEGS, and the existing relay resolve to distinct machines. After the
private component runbooks install pinned services,
`hetzner-acceptance.mjs` binds acceptance to that exact clean preflight
evidence, rechecks machine IDs and role markers, requires public HTTPS health,
and proves the exact expected set of concurrently active first-wave AEGSs. Pass
that set with `--expected-aegs github,jira,linear,gitlab,sentry,slack`. Its default mode
is read-only. `--execute-restarts` restarts only the exact AEDS and selected
AEGS systemd units and retains bounded, secret-free evidence. It never reboots
a host, changes firewall policy, removes containers, or prunes Docker.

```sh
node ./hetzner-acceptance.mjs \
  --preflight .artifacts/event-publication-hetzner/event-001/preflight.json \
  --run-id event-001 \
  --component github \
  --aeds-host root@aeds.example \
  --aegs-host root@aegs.example \
  --relay-host root@relay.example \
  --ssh-key ~/.ssh/arroba_event_staging \
  --aeds-url https://aeds.example \
  --aegs-url https://github-events.example
```

Acceptance accepts one shared `--ssh-key`, or the preferred host-specific
`--aeds-ssh-key` and `--aegs-ssh-key` pair. Key paths are used only for SSH and
are never retained in evidence.

The preflight accepts `--ssh-key` when every host shares an operator key, or
`--aeds-ssh-key`, `--aegs-ssh-key`, and (when a relay is checked)
`--relay-ssh-key` for the preferred host-specific credential layout. SSH key
paths are used only for the probes and are never written to evidence.

All service-specific containers, TLS examples, systemd units, migrations, and
operational scripts remain in their private component repositories.

Every production AEGS also exposes a capability-protected, authoritative
`PUT /v1/subscriptions/reconcile` endpoint. It persists logical event interests
separately from AEDS routes, so provider webhooks can be normalized to the correct
interest key without giving AEDS provider credentials or source-specific state.

# Temporary image builder cleanup

Run this guard on the existing OpenShip authority host, never on the image builder. It reads the existing root-owned mode-0600 `/etc/chariox-cloud/infrastructure-manager.env` privately and uses only its `CHARIOX_HETZNER_TOKEN`. It does not print or copy that token. This is a task-scoped cleanup guard, not a new provider runtime or provisioning service.

Create only a temporary CPX22 in fsn1 for already-compiled signed image preparation. The guard accepts a maximum two-hour deadline from the provider's exact server creation timestamp. A live availability/price check and root review precede provisioning. At the 2026-09-27 check, CPX22 costs EUR 0.037128/hour gross, plus EUR 0.000952/hour for primary IPv4. Two hours costs EUR 0.076160 before snapshot retention and any deletion delay. These prices must be rechecked when provisioning.

## Receipt

Persist create responses progressively in the private task directory so failure during creation cannot lose the firewall or key IDs. The guard consumes the final receipt only after all three resources exist. The creator owns exact-ID cleanup of any partial creation before this point; it must not start preparation until the full receipt and timer have been verified.

Store the following JSON at `/var/lib/chariox/image-builders/<runId>/receipt.json`, root-owned mode0600, with a root-owned mode0700 task directory and no writable or symbolic parent components. Values below describe the shape, not live resource IDs. Reject duplicate JSON keys.

```json
{
  "format": "chariox-image-builder-cleanup/v1",
  "runId": "chariox-image-c8f51c3b-20260927",
  "sourceCommit": "c8f51c3bdfb8d67633d4e97c353ac5b631e979a2",
  "expiresAt": "2026-09-27T22:00:00Z",
  "server": {
    "id": 1,
    "name": "chariox-image-c8f51c3b-20260927",
    "created": "2026-09-27T20:00:00Z",
    "ipv4": "192.0.2.1",
    "type": "cpx22",
    "location": "fsn1",
    "imageId": 387894169
  },
  "firewall": {"id": 2, "name": "chariox-image-c8f51c3b-20260927-ssh"},
  "sshKey": {
    "id": 3,
    "name": "chariox-image-c8f51c3b-20260927-key",
    "publicKeySha256": "<64 lowercase hex SHA256 of ssh-ed25519 SPACE base64, no comment/newline>"
  }
}
```

All three resources must have exactly these labels: `chariox.dev/managed-image-builder=true`, `chariox.dev/task=<runId>`, `chariox.dev/source-revision=<sourceCommit>`. The temporary firewall must have exactly one inbound TCP22 rule from `51.38.82.47/32`. Default outbound access permits package downloads. The server must have only this firewall, no attached volumes, no rescue/backups/delete protection, the exact receipt's base image, creation time, address and placement. Use a newly created SSH key exclusively for this builder. The guard checks its public fingerprint before deleting it.

## Register before preparation

Install the reviewed Python helper at `/usr/local/lib/chariox/image-builder-cleanup.py` as root-owned mode0755 and the service template at `/etc/systemd/system/chariox-image-builder-cleanup@.service` as root-owned mode0644. Validate the receipt and current remote identities with:

```sh
sudo python3 /usr/local/lib/chariox/image-builder-cleanup.py check \
  --receipt /var/lib/chariox/image-builders/chariox-image-c8f51c3b-20260927/receipt.json
```

Write a persistent, root-owned mode0644 timer file named `chariox-image-builder-cleanup-<runId>.timer` under `/etc/systemd/system`. Use the actual receipt deadline, not the example date:

```ini
[Unit]
Description=Deadline for exact temporary Chariox image builder
[Timer]
OnCalendar=2026-09-27 22:00:00 UTC
Persistent=true
AccuracySec=1s
Unit=chariox-image-builder-cleanup@chariox-image-c8f51c3b-20260927.service
[Install]
WantedBy=timers.target
```

```sh
sudo systemctl daemon-reload
sudo systemctl enable --now chariox-image-builder-cleanup-chariox-image-c8f51c3b-20260927.timer
sudo systemctl is-enabled chariox-image-builder-cleanup-chariox-image-c8f51c3b-20260927.timer
sudo systemctl show chariox-image-builder-cleanup-chariox-image-c8f51c3b-20260927.timer \
  --property=ActiveState --property=NextElapseUSecRealtime
```

Require `enabled`, `active` and the exact future deadline before starting preparation. Use an on-disk timer, not a transient `/run` timer that disappears on host reboot. The guard's `expire` mode independently rejects early execution. The service applies a 190-second hard execution bound and at most three starts in fifteen minutes. Root must monitor service failure; identity changes and provider API outages intentionally fail closed and can prevent a literal cost guarantee. Do not retry an ambiguous DELETE without fresh exact-ID readback. Each later service invocation performs that readback.

After successful snapshot publication, run `cleanup --receipt <same-file>` for immediate verified deletion, then read back all three resource IDs as absent. This mode permits cleanup before the deadline, but all identity checks still apply. Only then disable/remove the exact task timer and retain the receipt and output as evidence. The guard never deletes images/snapshots or primary IPs. The creator must verify that its temporary primary IPv4 has provider `auto_delete=true`; otherwise root must track and clean that exact unattached IP separately after server deletion. A stopped VM remains billable, so guest shutdown alone is not cleanup.

## Existing provider calls for the creator

Use the existing OpenShip host's private manager authority and Python standard-library HTTPS path already used for image preparation. No provider CLI installation or credential copy is required. The existing task scripts are examples of `urllib.request.Request` with an explicit method and bounded timeout; they contain old resource IDs and must not run unchanged.

The supported sequence is read-only GET `/server_types?name=cpx22`, `/images?name=ubuntu-26.04&type=system&architecture=x86`, and exact-name GETs to prove absence; then POST `/firewalls` with the one SSH rule and exact labels; POST `/ssh_keys` with the new public key and exact labels; and POST `/servers` with `server_type:cpx22`, `location:fsn1`, `image:387894169`, the returned firewall/key IDs, exact labels and `public_net:{enable_ipv4:true,enable_ipv6:false}`. Verify returned image, placement, address, key/firewall binding and primary IP auto-delete before arming the guard. Root's creator supplies cloud-init containing only the disposable image-builder marker and SSH hardening, not managed-worker enrollment. The marker is `/.chariox-managed-image-builder` with content `managed-remote-kernels-image-builder-v1` and mode0600.

No preparation, snapshot, deletion or other provider call is performed by this documentation or by the offline tests. Run `PYTHONDONTWRITEBYTECODE=1 python3 deploy/managed-kernel/image-builder-cleanup.test.py` for fake-provider regressions.

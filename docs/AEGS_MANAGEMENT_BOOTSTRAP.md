# AEGS management by the kernel identity key

A kernel needs nothing from Chariox Cloud, and no operator-issued token, to manage a
connection at an AEGS or to receive its events. The same path serves a Chariox-operated
gateway and a self-hosted one.

## Who the kernel is

The credential is the kernel's relay identity key (the P-256 key that already backs its
`kernel_public_key_thumbprint`). The kernel id at both gateways is the fingerprint of the
public key: the lowercase hex SHA-256 of the base64 public key. One id therefore names one key.
Signatures are ECDSA P-256 over SHA-256; every signed message starts with a domain tag so the
signature can never be mistaken for relay traffic, which uses the same key for key agreement.

## What the kernel sends

Every management request (`management protocol 5`) carries:

```text
x-chariox-kernel-key:        base64 SEC1 public key
x-chariox-kernel-timestamp:  unix milliseconds
x-chariox-kernel-nonce:      single-use random string (at most 128 bytes)
x-chariox-kernel-signature:  base64 r||s over the message below
x-chariox-owner-id:          the secret owner id the request acts for
```

The signed message is `chariox-aegs-request-v5`, method, path, generator id, owner id,
timestamp, nonce and the SHA-256 of the body, joined by NUL bytes. The AEGS accepts a request
only within five minutes of its clock, remembers each nonce for that window (a replay is refused),
and requires the body's `owner_id` to equal the signed header.

## The owner id

A connection belongs to an owner id: a secret random value the connection record carries and
that is never derived from public names. The first kernel that presents an owner id creates the
owner and is its first member. Another kernel of the same user that presents the same owner id
with its own signature joins the set and manages the same connection and receives the same
events, with no second provider login. A kernel holding a different owner id sees nothing of it:
a claim on a connection of another owner is answered like one on a connection that does not exist.
`POST /v1/owners/kernels/revoke` removes a kernel id from the set for good: its subscriptions
stop at once and presenting the owner id again does not bring it back. `GET /v1/owners/kernels`
lists the set (kernel ids are not secret).

## Admission

The operator chooses who may use the AEGS at all, with the same code for everyone
(`CHARIOX_AEGS_ADMISSION`): `open` (any kernel that signs, with per-kernel caps:
`CHARIOX_AEGS_OPEN_MAX_OWNERS_PER_KERNEL`, `CHARIOX_AEGS_OPEN_MAX_REQUESTS_PER_MINUTE`) or
`allowlist` (`CHARIOX_AEGS_ALLOWLIST_FILE`, a JSON object keyed by kernel id, read on every
request). `open` also caps the gateway as a whole (`CHARIOX_AEGS_OPEN_MAX_REQUESTS_PER_MINUTE_TOTAL`,
`CHARIOX_AEGS_OPEN_MAX_MEMBERSHIPS_TOTAL`, the kernel and owner pairs): the refusals are the typed
codes `gateway_rate_limited` (429) and `gateway_capacity` (503). A non-loopback AEGS must choose explicitly. Admission never decides who sees which
events: that is the owner id and the per-connection audience.

## What is gone

The Cloud-signed five-minute capability and its verification keys, `CHARIOX_AEGS_MANAGEMENT_TOKEN`
and the static management targets' `token` field no longer exist. A management target is only a
URL (`CHARIOX_AEGS_MANAGEMENT_TARGETS_FILE`); a URL discovered through the generator registry is
still limited to public addresses, which the kernel re-checks on every connection.

A static target (`CHARIOX_AEGS_MANAGEMENT_TARGETS_FILE`) is an explicit administrator action and keeps HTTPS or loopback HTTP
support for local and self-hosted deployments; Store metadata never enables that exception. The owner id is part of the
route binding ids that AEDS and the AEGS key subscriptions by.

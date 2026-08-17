# Hosted AEGS management bootstrap

Hosted kernels do not receive provider credentials or a long-lived AEGS token.
After reading a published catalog detail, the kernel sends the generator ID,
manifest digest, and registry management URL to Chariox Cloud using its existing
Cloud session or machine credential. Cloud verifies the kernel identity and
published metadata, derives the exact kernel and authenticated-user owner IDs,
and returns a short-lived Ed25519-signed capability scoped to that generator,
digest, kernel, owner set, user, URL, and `aegs-management` audience. The AEGS
compares the signed owner claim with the owner ID in every management request and
filters subscription listings to that owner, preventing a valid capability from
crossing tenants.

An SDK-based AEGS accepts either the existing static operator token (self-hosted
mode) or the capability. To enable hosted bootstrap, configure the AEGS with the
Cloud signing public key:

```text
CHARIOX_AEGS_MANAGEMENT_PUBLIC_KEY=<raw 32-byte Ed25519 key, base64url or hex>
CHARIOX_AEGS_MANAGEMENT_ISSUER=chariox-cloud
CHARIOX_AEGS_MANAGEMENT_URL=https://<the exact published AEGS management origin>
CHARIOX_AEGS_MANIFEST_DIGEST=sha256:<the exact published manifest digest>
```

Cloud operators configure the corresponding private key only in the Cloud API:

```text
CHARIOX_EVENT_GENERATOR_MANAGEMENT_SIGNING_KEY=<raw 32-byte Ed25519 key, base64 or hex>
CHARIOX_EVENT_GENERATOR_MANAGEMENT_KEY_ID=chariox-cloud-management-2026
CHARIOX_EVENT_GENERATOR_MANAGEMENT_ISSUER=chariox-cloud
```

The private key must never be submitted in a manifest, stored in the catalog,
placed in AEDS, or copied to a kernel. Capabilities expire after at most five
minutes and are not a replacement for provider authorization. A deployment can
continue to use `CHARIOX_AEGS_MANAGEMENT_TOKEN(_FILE)` for a self-hosted AEGS.

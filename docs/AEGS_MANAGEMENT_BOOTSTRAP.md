# Hosted AEGS management bootstrap

Hosted kernels do not receive provider credentials or a long-lived AEGS token.
After reading a published catalog detail, the kernel sends the generator ID,
manifest digest, and registry management URL to Chariox Cloud using its existing
Cloud session or machine credential. Cloud verifies the kernel identity and the
published metadata, then returns a short-lived Ed25519-signed capability scoped to
that generator, digest, kernel, user, URL, and `aegs-management` audience.

An SDK-based AEGS accepts either the existing static operator token (self-hosted
mode) or the capability. To enable hosted bootstrap, configure the AEGS with the
Cloud signing public key:

```text
CHARIOX_AEGS_MANAGEMENT_PUBLIC_KEY=<raw 32-byte Ed25519 key, base64url or hex>
CHARIOX_AEGS_MANAGEMENT_ISSUER=chariox-cloud
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

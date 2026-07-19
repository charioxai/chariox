import assert from "node:assert/strict"
import test from "node:test"

import {
  buildPublicationEgressPolicy,
  canonicalDnsName,
  isPublicUnicastAddress,
  publicationEgressDestination,
  validatePublicationEgressPolicy,
  validateResolvedPublicationAddresses,
} from "./policy.mjs"

test("publication egress policy is canonical, digest-bound, and exact", () => {
  const policy = buildPublicationEgressPolicy([
    { id: "provider:openai", host: "api.openai.com", ports: [443] },
    { id: "provider:anthropic", host: "api.anthropic.com", ports: [443] },
  ])
  assert.deepEqual(policy.destinations.map((destination) => destination.id), ["provider:anthropic", "provider:openai"])
  assert.equal(validatePublicationEgressPolicy(policy).policy_digest, policy.policy_digest)
  assert.equal(publicationEgressDestination(policy, "api.openai.com", 443).id, "provider:openai")
  assert.throws(() => publicationEgressDestination(policy, "metadata.google.internal", 443), /denied/)
  assert.throws(() => validatePublicationEgressPolicy({
    ...policy,
    destinations: [{ id: "provider:openai", host: "other.example.com", ports: [443] }],
  }), /digest does not match/)
})

test("publication egress policy rejects wildcards, IPs, noncanonical names, and non-TLS ports", () => {
  for (const host of ["*.example.com", "EXAMPLE.com", "127.0.0.1", "localhost", "example.com."]) {
    assert.throws(() => canonicalDnsName(host), /exact DNS|canonical lowercase/)
  }
  assert.throws(() => buildPublicationEgressPolicy([
    { id: "integration:http", host: "api.example.com", ports: [80] },
  ]), /TLS port 443 only/)
})

test("publication egress rejects every private, local, metadata, mapped, and reserved address", () => {
  const denied = [
    "0.0.0.0", "10.0.0.1", "100.64.0.1", "127.0.0.1", "169.254.169.254",
    "172.16.0.1", "192.168.1.1", "192.0.2.1", "198.18.0.1", "224.0.0.1", "255.255.255.255",
    "::", "::1", "::192.168.1.1", "::ffff:127.0.0.1", "64:ff9b::a00:1", "100::1", "2001:db8::1",
    "2002:a00:1::",
    "fc00::1", "fe80::1", "ff02::1",
  ]
  for (const address of denied) assert.equal(isPublicUnicastAddress(address), false, address)
  for (const address of ["8.8.8.8", "1.1.1.1", "2001:4860:4860::8888", "2606:4700:4700::1111"]) {
    assert.equal(isPublicUnicastAddress(address), true, address)
  }
})

test("one forbidden address poisons the complete DNS answer set", () => {
  assert.throws(() => validateResolvedPublicationAddresses([
    { address: "8.8.8.8", family: 4 },
    { address: "169.254.169.254", family: 4 },
  ]), /forbidden address/)
  assert.deepEqual(validateResolvedPublicationAddresses([
    { address: "8.8.8.8", family: 4 },
    { address: "8.8.8.8", family: 4 },
    { address: "2001:4860:4860::8888", family: 6 },
  ]), [
    { address: "8.8.8.8", family: 4 },
    { address: "2001:4860:4860::8888", family: 6 },
  ])
})

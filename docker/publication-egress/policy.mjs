import { createHash } from "node:crypto"
import { isIP } from "node:net"

export const PUBLICATION_EGRESS_POLICY_SCHEMA = "chariox.publication-egress-policy.v1"

const DNS_LABEL = /^[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?$/
const IPV4_DENY_RANGES = [
  ["0.0.0.0", 8],
  ["10.0.0.0", 8],
  ["100.64.0.0", 10],
  ["127.0.0.0", 8],
  ["169.254.0.0", 16],
  ["172.16.0.0", 12],
  ["192.0.0.0", 24],
  ["192.0.2.0", 24],
  ["192.88.99.0", 24],
  ["192.168.0.0", 16],
  ["198.18.0.0", 15],
  ["198.51.100.0", 24],
  ["203.0.113.0", 24],
  ["224.0.0.0", 4],
  ["240.0.0.0", 4],
].map(([address, prefix]) => [ipv4Integer(address), prefix])
const IPV6_DENY_RANGES = [
  ["::", 128],
  ["::1", 128],
  ["::", 96],
  ["::ffff:0:0", 96],
  ["64:ff9b::", 96],
  ["100::", 64],
  ["2001::", 23],
  ["2001:db8::", 32],
  ["2002::", 16],
  ["fc00::", 7],
  ["fe80::", 10],
  ["ff00::", 8],
].map(([address, prefix]) => [ipv6Integer(address), prefix])

export function buildPublicationEgressPolicy(destinations) {
  const policy = normalizePolicy({
    schema: PUBLICATION_EGRESS_POLICY_SCHEMA,
    mode: "enforced",
    default_action: "deny",
    destinations,
  })
  return {
    ...policy,
    policy_digest: publicationEgressPolicyDigest(policy),
  }
}

export function validatePublicationEgressPolicy(value) {
  const record = objectRecord(value, "publication egress policy")
  exactKeys(record, ["schema", "mode", "default_action", "destinations", "policy_digest"], "publication egress policy")
  if (record.schema !== PUBLICATION_EGRESS_POLICY_SCHEMA) throw new Error("publication egress policy schema is unsupported")
  if (record.mode !== "enforced" || record.default_action !== "deny") {
    throw new Error("publication egress policy must be enforced and deny by default")
  }
  if (typeof record.policy_digest !== "string" || !/^sha256:[a-f0-9]{64}$/.test(record.policy_digest)) {
    throw new Error("publication egress policy digest is invalid")
  }
  const normalized = normalizePolicy(record)
  const expectedDigest = publicationEgressPolicyDigest(normalized)
  if (record.policy_digest !== expectedDigest) throw new Error("publication egress policy digest does not match its destinations")
  return { ...normalized, policy_digest: expectedDigest }
}

export function publicationEgressPolicyDigest(value) {
  return `sha256:${createHash("sha256").update(canonicalJson(value)).digest("hex")}`
}

export function publicationEgressDestination(policy, host, port) {
  const normalizedHost = canonicalDnsName(host)
  if (!Number.isInteger(port) || port !== 443) throw new Error("publication egress permits TLS port 443 only")
  const destination = policy.destinations.find((candidate) => (
    candidate.host === normalizedHost && candidate.ports.includes(port)
  ))
  if (!destination) throw new Error("publication egress destination is denied")
  return destination
}

export function validateResolvedPublicationAddresses(addresses) {
  if (!Array.isArray(addresses) || addresses.length === 0) {
    throw new Error("publication egress destination did not resolve")
  }
  const unique = new Map()
  for (const candidate of addresses) {
    const address = typeof candidate === "string" ? candidate : candidate?.address
    const family = isIP(address)
    if (!family) throw new Error("publication egress resolver returned an invalid address")
    if (!isPublicUnicastAddress(address)) {
      throw new Error("publication egress DNS answer set contains a forbidden address")
    }
    unique.set(`${family}:${address}`, { address, family })
  }
  return [...unique.values()]
}

export function isPublicUnicastAddress(address) {
  const family = isIP(address)
  if (family === 4) {
    const value = ipv4Integer(address)
    return !IPV4_DENY_RANGES.some(([network, prefix]) => cidrContains(value, network, prefix, 32))
  }
  if (family === 6) {
    const value = ipv6Integer(address)
    return !IPV6_DENY_RANGES.some(([network, prefix]) => cidrContains(value, network, prefix, 128))
  }
  return false
}

export function canonicalDnsName(value) {
  if (typeof value !== "string" || value.length === 0 || value.length > 253 || value !== value.toLowerCase()) {
    throw new Error("publication egress host must be a canonical lowercase DNS name")
  }
  if (isIP(value) || value.endsWith(".") || value.includes("*") || value.includes("_")) {
    throw new Error("publication egress host must be an exact DNS name")
  }
  const labels = value.split(".")
  if (labels.length < 2 || labels.some((label) => !DNS_LABEL.test(label))) {
    throw new Error("publication egress host must be an exact DNS name")
  }
  return value
}

function normalizePolicy(value) {
  if (!Array.isArray(value.destinations) || value.destinations.length > 256) {
    throw new Error("publication egress destinations must be an array with at most 256 entries")
  }
  const destinations = value.destinations.map((candidate, index) => normalizeDestination(candidate, index))
    .sort((left, right) => left.id.localeCompare(right.id))
  const ids = new Set()
  const authorities = new Set()
  for (const destination of destinations) {
    if (ids.has(destination.id)) throw new Error("publication egress destination IDs must be unique")
    ids.add(destination.id)
    for (const port of destination.ports) {
      const authority = `${destination.host}:${port}`
      if (authorities.has(authority)) throw new Error("publication egress destination authorities must be unique")
      authorities.add(authority)
    }
  }
  return {
    schema: PUBLICATION_EGRESS_POLICY_SCHEMA,
    mode: "enforced",
    default_action: "deny",
    destinations,
  }
}

function normalizeDestination(value, index) {
  const record = objectRecord(value, `publication egress destination ${index}`)
  exactKeys(record, ["id", "host", "ports"], `publication egress destination ${index}`)
  if (typeof record.id !== "string" || !/^[a-z][a-z0-9:_-]{0,127}$/.test(record.id)) {
    throw new Error(`publication egress destination ${index} id is invalid`)
  }
  const host = canonicalDnsName(record.host)
  if (!Array.isArray(record.ports) || record.ports.length !== 1 || record.ports[0] !== 443) {
    throw new Error(`publication egress destination ${index} must permit TLS port 443 only`)
  }
  return { id: record.id, host, ports: [443] }
}

function canonicalJson(value) {
  if (Array.isArray(value)) return `[${value.map(canonicalJson).join(",")}]`
  if (value && typeof value === "object") {
    return `{${Object.keys(value).sort().map((key) => `${JSON.stringify(key)}:${canonicalJson(value[key])}`).join(",")}}`
  }
  return JSON.stringify(value)
}

function objectRecord(value, label) {
  if (!value || typeof value !== "object" || Array.isArray(value)) throw new Error(`${label} must be an object`)
  return value
}

function exactKeys(record, keys, label) {
  const expected = new Set(keys)
  if (Object.keys(record).some((key) => !expected.has(key)) || keys.some((key) => !(key in record))) {
    throw new Error(`${label} fields are invalid`)
  }
}

function ipv4Integer(address) {
  const parts = String(address).split(".").map(Number)
  if (parts.length !== 4 || parts.some((part) => !Number.isInteger(part) || part < 0 || part > 255)) {
    throw new Error("invalid IPv4 address")
  }
  return parts.reduce((value, part) => (value << 8n) | BigInt(part), 0n)
}

function ipv6Integer(address) {
  let source = String(address).toLowerCase()
  if (source.includes("%")) throw new Error("IPv6 zone identifiers are forbidden")
  if (source.includes(".")) {
    const lastColon = source.lastIndexOf(":")
    const ipv4 = ipv4Integer(source.slice(lastColon + 1))
    source = `${source.slice(0, lastColon)}:${Number((ipv4 >> 16n) & 0xffffn).toString(16)}:${Number(ipv4 & 0xffffn).toString(16)}`
  }
  const halves = source.split("::")
  if (halves.length > 2) throw new Error("invalid IPv6 address")
  const left = halves[0] ? halves[0].split(":") : []
  const right = halves[1] ? halves[1].split(":") : []
  const missing = 8 - left.length - right.length
  if ((halves.length === 1 && missing !== 0) || (halves.length === 2 && missing < 1)) {
    throw new Error("invalid IPv6 address")
  }
  const parts = [...left, ...Array(missing).fill("0"), ...right]
  if (parts.length !== 8 || parts.some((part) => !/^[a-f0-9]{1,4}$/.test(part))) {
    throw new Error("invalid IPv6 address")
  }
  return parts.reduce((value, part) => (value << 16n) | BigInt(`0x${part}`), 0n)
}

function cidrContains(value, network, prefix, bits) {
  const shift = BigInt(bits - prefix)
  return (value >> shift) === (network >> shift)
}

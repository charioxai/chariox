#!/usr/bin/env node
// Signs the small release documents on the signing Mac. Nothing large crosses: it reads the
// signing requests (a few KB each) and writes detached signatures, public pins and a receipt.
// Needs only this file and release-signing-documents.mjs, no checkout of the release and no artifacts.
import { createPrivateKey } from "node:crypto"
import { mkdir, readFile, stat, writeFile } from "node:fs/promises"
import { basename, join, resolve } from "node:path"
import { keyFingerprint, parseSigningRequest, rawPublicKey, sha, signRequest } from "./release-signing-documents.mjs"

const USAGE =
  "usage: node sign-digests-on-mac.mjs --request <file>@sha256:<digest> [--request ...] --release-key <ed25519.pem> [--builder-key <ed25519.pem>] --output <new directory>"

function parseArguments(argv) {
  const options = { requests: [], testAllowNonMac: false }
  for (let index = 0; index < argv.length; index += 1) {
    const flag = argv[index]
    if (flag === "--test-allow-non-mac") {
      options.testAllowNonMac = true
      continue
    }
    const value = argv[++index]
    if (!value || value.startsWith("--")) throw new Error(USAGE)
    if (flag === "--request") options.requests.push(value)
    else if (flag === "--release-key") options.releaseKey = value
    else if (flag === "--builder-key") options.builderKey = value
    else if (flag === "--output") options.output = value
    else throw new Error(USAGE)
  }
  if (!options.requests.length || !options.releaseKey || !options.output) throw new Error(USAGE)
  return options
}

// The real flow cannot reach the test path by accident: the flag is explicit, and it only accepts
// keys whose file name ends in .throwaway.pem, so a release or builder key is refused off the Mac.
function requireMacOrThrowawayTest(options) {
  if (process.platform === "darwin") {
    if (options.testAllowNonMac) throw new Error("--test-allow-non-mac is for throwaway keys off the Mac; do not pass it on the signing Mac")
    return
  }
  if (!options.testAllowNonMac) throw new Error("Run this signing command on the coordinator Mac only")
  for (const path of [options.releaseKey, options.builderKey].filter(Boolean)) {
    if (!path.endsWith(".throwaway.pem")) throw new Error(`--test-allow-non-mac only accepts keys named *.throwaway.pem, not ${basename(path)}`)
  }
  process.stderr.write("TEST MODE: signing off the Mac with throwaway keys; the output is not a release\n")
}

async function loadKey(path, label) {
  if ((await stat(path)).mode & 0o077) throw new Error(`${label} key must have mode 0600`)
  const key = createPrivateKey(await readFile(path))
  if (key.asymmetricKeyType !== "ed25519") throw new Error(`${label} key must be Ed25519`)
  return key
}

const options = parseArguments(process.argv.slice(2))
requireMacOrThrowawayTest(options)

const parsed = []
for (const spec of options.requests) {
  const at = spec.lastIndexOf("@")
  if (at < 1) throw new Error(`--request must be <file>@sha256:<digest>, got ${spec}`)
  const request = parseSigningRequest(await readFile(spec.slice(0, at)), spec.slice(at + 1))
  if (parsed.some((other) => other.request.kind === request.request.kind)) throw new Error(`two ${request.request.kind} requests given`)
  parsed.push(request)
}
const identity = parsed[0].request
for (const { request } of parsed) {
  if (request.sourceCommit !== identity.sourceCommit || request.sourceTree !== identity.sourceTree) {
    throw new Error("the signing requests are for different source commits")
  }
}
const keys = { release: await loadKey(options.releaseKey, "release") }
if (parsed.some(({ request }) => request.kind === "managed")) {
  if (!options.builderKey) throw new Error("a managed request needs --builder-key (pass the release key path twice if one key holds both roles)")
  keys.builder = await loadKey(options.builderKey, "builder")
}

const output = resolve(options.output)
await mkdir(output, { recursive: false, mode: 0o755 })
const receipt = {
  schemaVersion: 1,
  signedAt: new Date().toISOString(),
  testMode: options.testAllowNonMac,
  sourceCommit: identity.sourceCommit,
  sourceTree: identity.sourceTree,
  keys: Object.fromEntries(
    Object.entries(keys).map(([role, key]) => [role, { publicKey: rawPublicKey(key).toString("base64"), fingerprint: keyFingerprint(rawPublicKey(key)) }]),
  ),
  releases: {},
}
for (const document of parsed) {
  const { request } = document
  const directory = join(output, request.kind)
  await mkdir(directory, { mode: 0o755 })
  const files = signRequest(document, keys)
  for (const [name, bytes] of files) await writeFile(join(directory, name), bytes, { flag: "wx", mode: 0o644 })
  receipt.releases[request.kind] = {
    requestDigest: sha(Buffer.from(JSON.stringify(request) + "\n")),
    attestationDigest: request.attestation.sha256,
    attestationSignatureDigest: sha(files.get("attestation.sig")),
    builderPublicKeyDigest: sha(files.get("builder-public-key")),
    manifestDigest: sha(files.get("release-manifest.json")),
    manifestSignatureDigest: sha(files.get("release-manifest.sig")),
  }
}
await writeFile(join(output, "SIGNING_RECEIPT.json"), JSON.stringify(receipt, null, 2) + "\n", { flag: "wx", mode: 0o644 })
process.stdout.write(JSON.stringify(receipt, null, 2) + "\n")

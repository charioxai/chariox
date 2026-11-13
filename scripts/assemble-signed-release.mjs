#!/usr/bin/env node
// Embeds the signer's detached signatures and public pins into a staged release on the builder, then
// re-verifies every signature and every artifact digest from disk. Refuses on any mismatch.
import { chmod, lstat, readFile, utimes, writeFile } from "node:fs/promises"
import { dirname, join, resolve } from "node:path"
import { parseSigningRequest, SIGNED_FILES, sha, verifySignedFiles } from "./release-signing-documents.mjs"
import { sha256File, sha256Tree } from "./release-tree-digest.mjs"

const USAGE =
  "usage: node assemble-signed-release.mjs --kind boot|managed --tree <staged output directory> --request <signing request> --signed <signer output directory> --release-public-key <base64> [--builder-public-key <base64>]"

// Where each signed document lives in a staged release, and where an artifact entry lives.
const LAYOUTS = {
  boot: {
    artifact: (tree, entry) => join(tree, "artifacts", entry.name),
    embedded: (tree) => ({
      "attestation.sig": join(tree, "artifacts/chariox-build-attestation-signature"),
      "builder-public-key": join(tree, "artifacts/chariox-builder-public-key"),
      "release-manifest.json": join(tree, "release-manifest.json"),
      "release-manifest.sig": join(tree, "release-manifest.sig"),
    }),
  },
  managed: {
    artifact: (tree, entry) => join(tree, "rootfs", entry.path),
    embedded: (tree) => ({
      "attestation.sig": join(tree, "rootfs/usr/lib/chariox/build-attestation.sig"),
      "builder-public-key": join(tree, "rootfs/usr/lib/chariox/builder-public-key"),
      "release-manifest.json": join(tree, "rootfs/usr/lib/chariox/release-manifest.json"),
      "release-manifest.sig": join(tree, "rootfs/usr/lib/chariox/release-manifest.sig"),
      "release-public-key": join(tree, "rootfs/usr/lib/chariox/release-public-key"),
    }),
  },
}

function parseArguments(argv) {
  const options = {}
  for (let index = 0; index < argv.length; index += 2) {
    const flag = argv[index]
    const value = argv[index + 1]
    if (!flag?.startsWith("--") || !value || value.startsWith("--") || flag.slice(2) in options) throw new Error(USAGE)
    options[flag.slice(2)] = value
  }
  if (!LAYOUTS[options.kind] || !options.tree || !options.request || !options.signed || !options["release-public-key"]) throw new Error(USAGE)
  if (options.kind === "managed" && !options["builder-public-key"]) throw new Error("a managed release needs --builder-public-key")
  return options
}

async function artifactDigest(path, label) {
  const metadata = await lstat(path).catch(() => {
    throw new Error(`artifact ${label} is missing from the staged release at ${path}`)
  })
  if (metadata.isSymbolicLink()) throw new Error(`artifact ${label} is a symlink`)
  return metadata.isDirectory() ? sha256Tree(path) : sha256File(path)
}

async function checkArtifacts(kind, tree, entries) {
  for (const entry of entries) {
    const actual = await artifactDigest(LAYOUTS[kind].artifact(tree, entry), entry.name)
    if (actual !== entry.sha256) throw new Error(`artifact ${entry.name} (${entry.path}) has digest ${actual}, the signed manifest has ${entry.sha256}`)
  }
}

async function readSigned(kind, paths, pins) {
  const files = new Map()
  for (const [name, path] of Object.entries(paths)) files.set(name, await readFile(path))
  // Boot releases carry no release public key file; the pin stands in for it.
  if (!files.has("release-public-key")) files.set("release-public-key", Buffer.from(pins.release + (kind === "boot" ? "\n" : "")))
  return files
}

const options = parseArguments(process.argv.slice(2))
const { kind } = options
const tree = resolve(options.tree)
const pins = { release: options["release-public-key"], builder: options["builder-public-key"] }
const receipt = JSON.parse(await readFile(join(options.signed, "SIGNING_RECEIPT.json")))
const signedRelease = receipt.releases?.[kind]
if (!signedRelease) throw new Error(`the signer output has no ${kind} release`)
const document = parseSigningRequest(await readFile(options.request), signedRelease.requestDigest)
if (document.request.kind !== kind) throw new Error(`the signing request is a ${document.request.kind} request, not ${kind}`)

const signed = new Map()
for (const name of SIGNED_FILES) signed.set(name, await readFile(join(options.signed, kind, name)))
for (const [field, name] of [["attestationSignatureDigest", "attestation.sig"], ["builderPublicKeyDigest", "builder-public-key"], ["manifestDigest", "release-manifest.json"], ["manifestSignatureDigest", "release-manifest.sig"]]) {
  if (sha(signed.get(name)) !== signedRelease[field]) throw new Error(`${kind} ${name} does not match the digest in SIGNING_RECEIPT.json`)
}
verifySignedFiles(document, signed, pins)
await checkArtifacts(kind, tree, document.request.entries.filter((entry) => entry.sha256))

const timestamp = new Date(Number(process.env.SOURCE_DATE_EPOCH ?? "0") * 1000)
const embedded = LAYOUTS[kind].embedded(tree)
for (const [name, path] of Object.entries(embedded)) {
  await writeFile(path, signed.get(name), { flag: "wx", mode: 0o644 }).catch((error) => {
    throw new Error(error.code === "EEXIST" ? `${path} already exists; assemble into a freshly staged release` : error.message)
  })
  await chmod(path, 0o644)
  await utimes(path, timestamp, timestamp)
}
for (const directory of new Set(Object.values(embedded).map((path) => dirname(path)))) await utimes(directory, timestamp, timestamp)

// End to end, from what is now on disk.
const finalEntries = verifySignedFiles(document, await readSigned(kind, embedded, pins), pins)
await checkArtifacts(kind, tree, finalEntries)
process.stdout.write(`${JSON.stringify({ kind, manifestDigest: signedRelease.manifestDigest, artifacts: finalEntries.length, sourceCommit: document.request.sourceCommit })}\n`)

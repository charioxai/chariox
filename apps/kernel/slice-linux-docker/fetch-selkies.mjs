#!/usr/bin/env node
import { createHash, randomUUID } from "node:crypto"
import { createReadStream, createWriteStream } from "node:fs"
import { mkdir, readFile, rename, rm } from "node:fs/promises"
import { basename, resolve } from "node:path"
import { Readable, Transform } from "node:stream"
import { pipeline } from "node:stream/promises"

const lock = JSON.parse(await readFile(new URL("./selkies.lock.json", import.meta.url), "utf8"))
const [mode, destination, architecture] = process.argv.slice(2)
const usage = "usage: fetch-selkies.mjs source|runtime|licenses OUTPUT_DIRECTORY [amd64|arm64]"
if (!destination || !["source", "runtime", "licenses"].includes(mode)) throw new Error(usage)
if (mode === "runtime" && !["amd64", "arm64"].includes(architecture)) throw new Error(usage)
if (lock.schema_version !== 1) throw new Error("unsupported Selkies lock schema")

const artifacts = mode === "source"
  ? [lock.selkies.source]
  : mode === "licenses"
    ? [lock.selkies.source, lock.pixelflux.source, lock.pcmflux.source]
    : [lock.pixelflux.wheels[architecture], lock.pcmflux.wheels[architecture]]
const outputDirectory = resolve(destination)
await mkdir(outputDirectory, { recursive: true })
for (const artifact of artifacts) await downloadVerified(artifact)

async function digestFile(filename) {
  const hash = createHash("sha256")
  for await (const chunk of createReadStream(filename)) hash.update(chunk)
  return hash.digest("hex")
}

async function downloadVerified(artifact) {
  if (basename(artifact.filename) !== artifact.filename || !/^[a-f0-9]{64}$/.test(artifact.sha256)) {
    throw new Error("invalid artifact filename or SHA-256 in Selkies lock")
  }
  const url = new URL(artifact.url)
  if (url.protocol !== "https:" || url.hostname !== "github.com" || !url.pathname.startsWith("/selkies-project/")) {
    throw new Error("Selkies artifacts must come from the pinned upstream GitHub repositories")
  }
  const output = resolve(outputDirectory, artifact.filename)
  try {
    if (await digestFile(output) === artifact.sha256) {
      console.log(`verified cached ${artifact.filename}`)
      return
    }
    throw new Error(`cached ${artifact.filename} failed SHA-256 verification; remove that file and retry`)
  } catch (error) {
    if (error.code !== "ENOENT") throw error
  }

  const temporary = `${output}.${randomUUID()}.part`
  try {
    const response = await fetch(url, { signal: AbortSignal.timeout(120_000) })
    if (!response.ok || !response.body) throw new Error(`download failed: HTTP ${response.status}`)
    if (new URL(response.url).protocol !== "https:") throw new Error("artifact download redirected away from HTTPS")
    const hash = createHash("sha256")
    let bytes = 0
    const verify = new Transform({
      transform(chunk, _encoding, callback) {
        bytes += chunk.length
        if (bytes > (artifact.size ?? 32 * 1024 * 1024)) {
          callback(new Error(`download exceeded the pinned size for ${artifact.filename}`))
          return
        }
        hash.update(chunk)
        callback(null, chunk)
      },
    })
    await pipeline(Readable.fromWeb(response.body), verify, createWriteStream(temporary, { flags: "wx" }))
    if (artifact.size !== undefined && bytes !== artifact.size) throw new Error(`size mismatch for ${artifact.filename}`)
    if (hash.digest("hex") !== artifact.sha256) throw new Error(`SHA-256 mismatch for ${artifact.filename}`)
    await rename(temporary, output)
    console.log(`verified ${artifact.filename} (${bytes} bytes)`)
  } finally {
    await rm(temporary, { force: true })
  }
}

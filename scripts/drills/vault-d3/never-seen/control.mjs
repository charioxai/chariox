// Negative control (NS-4): plant a random needle in each file type and prove the scanner finds it. A scan whose control
// finds nothing is INVALID, never clean.
import { execFile } from "node:child_process"
import { randomBytes } from "node:crypto"
import { mkdtemp, rm, writeFile } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"
import { promisify } from "node:util"
import { needleSet } from "./needles.mjs"
import { ocrAvailable, scanRoots } from "./scan.mjs"

const run = promisify(execFile)
export const controlValue = () => "CONTROL_" + randomBytes(12).toString("hex")

async function pngWithText(text) {
  const { stdout } = await run("convert", ["-size", "900x120", "xc:white", "-fill", "black", "-pointsize", "40", "-gravity", "center", "-annotate", "0", text, "png:-"], { encoding: "buffer", maxBuffer: 8 * 1024 * 1024 })
  return stdout
}

/** Returns {types: {type: bool}, ocr: bool|null}. Files are written to a scratch dir that holds only control values. */
export async function syntheticControl({ ocr = false } = {}) {
  const value = controlValue(), needles = needleSet(value)
  const dir = await mkdtemp(join(tmpdir(), "never-seen-control-"))
  try {
    const raw = Buffer.from(value)
    const files = {
      jsonl: Buffer.from(JSON.stringify({ type: "tool_result", text: `x ${value} y` }) + "\n"),
      ndjson: Buffer.from(JSON.stringify({ level: "info", fields: { note: value } }) + "\n"),
      json: Buffer.from(JSON.stringify({ a: [value] })),
      log: Buffer.from(`line one\nline ${value}\n`),
      "json-escaped": Buffer.from(JSON.stringify({ a: JSON.stringify({ v: value + '"' }) })),
      "base64-field": Buffer.from(JSON.stringify({ blob: Buffer.from("prefix:" + value).toString("base64") })),
      // A binary file larger than the read chunk with the needle straddling the chunk boundary.
      "chunk-boundary": Buffer.concat([Buffer.alloc(4 * 1024 * 1024 - 7, 0x41), raw, Buffer.alloc(100, 0x42)]),
    }
    const ext = { jsonl: "jsonl", ndjson: "ndjson", json: "json", log: "log", "json-escaped": "json", "base64-field": "json", "chunk-boundary": "db" }
    const nameOf = type => `${type}.${ext[type]}`
    for (const [type, data] of Object.entries(files)) await writeFile(join(dir, nameOf(type)), data)
    const { roots } = await scanRoots([{ target: "control", kind: "dir", path: dir }], needles, { ocr: false })
    const hitFiles = new Set(roots[0].hits.map(h => h.path.split("/").pop()))
    const types = {}
    for (const type of Object.keys(files)) types[type] = hitFiles.has(nameOf(type))
    let ocrOk = null
    if (ocr) {
      if (!(await ocrAvailable())) ocrOk = false
      else {
        const png = await pngWithText(value)
        await writeFile(join(dir, "shot.png"), png)
        await writeFile(join(dir, "provider.jsonl"), JSON.stringify({ content: [{ type: "image", source: { type: "base64", media_type: "image/png", data: png.toString("base64") } }] }) + "\n")
        const imgs = await scanRoots([{ target: "control", kind: "dir", path: dir }], needles, { ocr: true })
        const viaOcr = imgs.roots[0].hits.filter(h => h.via === "ocr").map(h => h.path.split("/").pop())
        types["png-ocr"] = viaOcr.includes("shot.png")
        types["jsonl-image-ocr"] = viaOcr.includes("provider.jsonl")
        ocrOk = types["png-ocr"] && types["jsonl-image-ocr"]
      }
    }
    return { types, ocr: ocrOk }
  } finally { await rm(dir, { recursive: true, force: true }) }
}

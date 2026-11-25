// Scanner for the "never seen" cells (HARNESS.md section 1). Reports counts and file+offset only, never a value.
import { execFile } from "node:child_process"
import { lstat, open, readdir, readFile } from "node:fs/promises"
import { join } from "node:path"
import { promisify } from "node:util"

const run = promisify(execFile)
const SKIP_NAME = /^(vault|config|auth\.json|credentials)(\..*)?$|\.(token|key|pem)$/i
const CHUNK = 4 * 1024 * 1024
const IMAGE_BLOCK = /"(?:data|image_data|base64)"\s*:\s*"([A-Za-z0-9+/=]{2000,})"/g

export const skipped = path => SKIP_NAME.test(path.split("/").pop())

/** Count needle hits in one buffer-streamed file. Hits carry form name and byte offset only. */
export async function scanFile(path, needles, { maxHits = 20 } = {}) {
  const handle = await open(path, "r")
  const hits = []
  let bytes = 0
  try {
    const overlap = Math.max(...needles.map(n => n.bytes.length)) - 1
    let carry = Buffer.alloc(0), base = 0
    for (;;) {
      const buffer = Buffer.alloc(CHUNK)
      const { bytesRead } = await handle.read(buffer, 0, CHUNK, bytes)
      if (bytesRead === 0) break
      bytes += bytesRead
      const window = Buffer.concat([carry, buffer.subarray(0, bytesRead)])
      for (const needle of needles) {
        let from = 0
        for (;;) {
          const at = window.indexOf(needle.bytes, from)
          if (at < 0) break
          // A hit fully inside the carried overlap was already counted by the previous window.
          if (at + needle.bytes.length > carry.length) hits.push({ form: needle.name, offset: base + at })
          from = at + 1
          if (hits.length >= maxHits) return { hits, bytes }
        }
      }
      carry = window.subarray(Math.max(0, window.length - overlap))
      base += window.length - carry.length
    }
  } finally { await handle.close() }
  return { hits, bytes }
}

async function* walk(path) {
  const info = await lstat(path).catch(() => null)
  if (!info || info.isSymbolicLink() || skipped(path)) return
  if (info.isFile()) { yield { path, size: info.size }; return }
  if (!info.isDirectory()) return
  for (const entry of await readdir(path)) yield* walk(join(path, entry))
}

/** tesseract through stdin/stdout so an OCR hit never writes a second copy of a value to disk. */
export async function ocrAvailable() { try { await run("tesseract", ["--version"]); return true } catch { return false } }
export async function ocrBuffer(png) {
  const child = execFile("tesseract", ["stdin", "stdout", "--psm", "11"], { maxBuffer: 16 * 1024 * 1024, encoding: "utf8" })
  const done = new Promise((resolve, reject) => { let out = ""; child.stdout.on("data", c => { out += c }); child.on("error", reject); child.on("close", () => resolve(out)) })
  child.stdin.end(png)
  return done
}

const isImage = buffer => buffer.subarray(0, 4).equals(Buffer.from([0x89, 0x50, 0x4e, 0x47])) || buffer.subarray(0, 3).equals(Buffer.from([0xff, 0xd8, 0xff]))

function textHits(text, needles) {
  const hay = Buffer.from(text, "utf8"), hits = []
  for (const needle of needles) { let from = 0; for (;;) { const at = hay.indexOf(needle.bytes, from); if (at < 0) break; hits.push({ form: needle.name, offset: at }); from = at + 1 } }
  return hits
}

/** Images: png/jpeg files and base64 image blocks inside json/jsonl. Returns counts; OCR text is matched in memory. */
async function scanImages(path, size, needles, ocr) {
  const out = { images: 0, ocr_ok: 0, not_scanned: 0, hits: [] }
  const feed = async (buffer, where) => {
    out.images++
    if (!ocr.available) { out.not_scanned++; return }
    const text = await ocrBuffer(buffer).catch(() => null)
    if (text === null) { out.not_scanned++; return }
    out.ocr_ok++
    for (const hit of textHits(text.replace(/\s+/g, ""), needles)) out.hits.push({ ...hit, offset: where, via: "ocr" })
  }
  if (/\.(png|jpe?g)$/i.test(path)) { await feed(await readFile(path), 0); return out }
  if (/\.(jsonl|json|ndjson)$/i.test(path) && size <= 256 * 1024 * 1024) {
    const text = await readFile(path, "utf8")
    for (const match of text.matchAll(IMAGE_BLOCK)) {
      const buffer = Buffer.from(match[1], "base64")
      if (isImage(buffer)) await feed(buffer, match.index)
    }
  }
  return out
}

/** /proc/<pid>/cmdline and environ of every readable process: argv and environment must not hold the value either. */
async function* procFiles() {
  for (const pid of (await readdir("/proc").catch(() => [])).filter(n => /^\d+$/.test(n))) {
    for (const name of ["cmdline", "environ"]) yield { path: `/proc/${pid}/${name}`, pid: Number(pid), name }
  }
}

async function scanProc(needles) {
  const result = { files: 0, bytes: 0, hits: [] }
  for await (const entry of procFiles()) {
    let data
    try { data = await readFile(entry.path) } catch { continue }
    result.files++; result.bytes += data.length
    for (const needle of needles) if (data.includes(needle.bytes)) result.hits.push({ form: needle.name, offset: 0, path: `/proc/<pid ${entry.pid}>/${entry.name}` })
  }
  return result
}

/**
 * roots: [{target, kernel, path, kind: "dir"|"file"|"proc"}]. Returns per-root {files, bytes, hits[]} plus image counts.
 * No value and no matched line is returned: hits are {form, offset, path} only.
 */
export async function scanRoots(roots, needles, { ocr = false } = {}) {
  const ocrState = { available: ocr ? await ocrAvailable() : false }
  const report = []
  for (const root of roots) {
    const entry = { target: root.target, kernel: root.kernel ?? "", path: root.kind === "proc" ? "/proc" : root.path, files: 0, bytes: 0, hits: [], images: 0, ocr_ok: 0, not_scanned: 0 }
    if (root.kind === "proc") {
      const proc = await scanProc(needles)
      Object.assign(entry, { files: proc.files, bytes: proc.bytes, hits: proc.hits })
    } else {
      for await (const file of walk(root.path)) {
        const found = await scanFile(file.path, needles).catch(() => null)
        if (!found) continue
        entry.files++; entry.bytes += found.bytes
        for (const hit of found.hits) entry.hits.push({ ...hit, path: file.path })
        if (ocr) {
          const images = await scanImages(file.path, file.size, needles, ocrState)
          entry.images += images.images; entry.ocr_ok += images.ocr_ok; entry.not_scanned += images.not_scanned
          for (const hit of images.hits) entry.hits.push({ ...hit, path: file.path })
        }
      }
    }
    report.push(entry)
  }
  return { roots: report, ocrAvailable: ocrState.available }
}

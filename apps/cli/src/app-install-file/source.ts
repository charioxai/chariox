import { createHash } from "node:crypto"
import { constants, type BigIntStats } from "node:fs"
import { lstat, open, type FileHandle } from "node:fs/promises"
import { extname, resolve } from "node:path"

export const chunkBytes = 512 * 1024
export const maxArchiveBytes = 128 * 1024 * 1024
export class InstallCancelled extends Error { constructor() { super("App installation cancelled") } }
export class InstallFileChanged extends Error { constructor() { super("The selected App file changed. Cancel this attempt and install the finished file again.") } }
export function checkCancelled(cancelled: () => boolean): void { if (cancelled()) throw new InstallCancelled() }
function same(a: BigIntStats, b: BigIntStats): boolean {
  return a.dev === b.dev && a.ino === b.ino && a.size === b.size && a.mtimeNs === b.mtimeNs && a.ctimeNs === b.ctimeNs
}

/** One held local descriptor; nothing in this type crosses the kernel wire. */
export class AppFileSource {
  private constructor(readonly path: string, private file: FileHandle, private identity: BigIntStats,
    readonly size: number, readonly digest: string, private chunks: string[]) {}

  static async open(selected: string, cwd: string, cancelled: () => boolean, progress: (bytes: number, total: number) => void): Promise<AppFileSource> {
    if (!selected || selected.includes("\0") || Buffer.byteLength(selected) > 4096 || extname(selected).toLowerCase() !== ".cxapp") throw new Error("Choose a .cxapp file to install")
    const path = resolve(cwd, selected)
    const file = await open(path, constants.O_RDONLY | constants.O_NOFOLLOW | constants.O_NONBLOCK)
    try {
      const identity = await file.stat({ bigint: true })
      if (!identity.isFile() || identity.size <= 0n || identity.size > BigInt(maxArchiveBytes)) throw new Error("App packages must be regular files between 1 byte and 128 MiB")
      const size = Number(identity.size)
      const hash = createHash("sha256")
      const chunks: string[] = []
      for (let offset = 0; offset < size; offset += chunkBytes) {
        checkCancelled(cancelled)
        const bytes = await read(file, offset, Math.min(chunkBytes, size - offset))
        hash.update(bytes)
        chunks.push(digest(bytes))
        progress(offset + bytes.length, size)
      }
      const source = new AppFileSource(path, file, identity, size, `sha256:${hash.digest("hex")}`, chunks)
      await source.unchanged()
      checkCancelled(cancelled)
      return source
    } catch (error) { await file.close(); throw error }
  }

  async chunk(offset: number): Promise<{ bytes: Buffer; sha256: string }> {
    if (!Number.isSafeInteger(offset) || offset < 0 || offset >= this.size || offset % chunkBytes !== 0) throw new Error("Invalid App upload offset")
    await this.unchanged()
    const bytes = await read(this.file, offset, Math.min(chunkBytes, this.size - offset))
    const sha256 = digest(bytes)
    if (sha256 !== this.chunks[offset / chunkBytes]) throw new InstallFileChanged()
    return { bytes, sha256 }
  }
  async unchanged(): Promise<void> {
    if (!same(this.identity, await this.file.stat({ bigint: true })) || !same(this.identity, await lstat(this.path, { bigint: true }))) throw new InstallFileChanged()
  }
  async close(): Promise<void> { await this.file.close() }
}
function digest(bytes: Uint8Array): string { return `sha256:${createHash("sha256").update(bytes).digest("hex")}` }
async function read(file: FileHandle, offset: number, length: number): Promise<Buffer> {
  const bytes = Buffer.allocUnsafe(length)
  let read = 0
  while (read < length) {
    const result = await file.read(bytes, read, length - read, offset + read)
    if (!result.bytesRead) throw new InstallFileChanged()
    read += result.bytesRead
  }
  return bytes
}

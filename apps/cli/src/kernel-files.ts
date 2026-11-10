import { createHash } from "node:crypto"
import { constants, createReadStream, type Stats } from "node:fs"
import { resolve } from "node:path"
import { link, lstat, open, rename, rm, stat, type FileHandle } from "node:fs/promises"
import { LocalIpcError } from "@chariox/kernel-client/ipc"
import { CommandFailure } from "./agent-command-table.js"
import { terminalCwd } from "./app-install-file.js"

// Matches the kernel: a chunk travels base64 inside one relayed request of at most 768 KiB.
const CHUNK_BYTES = 256 * 1024
// The same name the kernel gives an unfinished upload, so a download resumes the way an upload does.
const PARTIAL_SUFFIX = ".chariox-partial"

export const FILES_VERBS = ["get", "put"] as const
export type FilesVerb = typeof FILES_VERBS[number]
export type SendRequest = (body: Record<string, unknown>) => Promise<unknown>

type Chunk = { data_base64: string; size: number }
type Transfer = { path: string; size: number; sha256: string; bytes_sent_or_received: number; resumed_from: number; elapsed_ms: number }

async function readRemote(send: SendRequest, path: string, offset: number, length: number): Promise<Chunk> {
  return (await send({ ReadKernelFile: { path, offset, length } }) as { KernelFileChunk: Chunk }).KernelFileChunk
}

// A read of length 0 moves no bytes and reports the file's size, so on the unfinished upload's
// name it says where the upload continues. No such file means it starts at 0.
async function remotePartialSize(send: SendRequest, remotePath: string): Promise<number> {
  try { return (await readRemote(send, remotePath + PARTIAL_SUFFIX, 0, 0)).size } catch (error) {
    if (error instanceof LocalIpcError && error.code === "file_not_found") return 0
    throw error
  }
}

async function statLocal(path: string): Promise<Stats | null> {
  try { return await stat(path) } catch (error) {
    if ((error as NodeJS.ErrnoException).code === "ENOENT") return null
    throw error
  }
}

async function sha256Of(path: string): Promise<string> {
  const hash = createHash("sha256")
  for await (const chunk of createReadStream(path)) hash.update(chunk)
  return hash.digest("hex")
}

// A replaced file keeps its permissions (a 0600 credentials file stays private); a new file gets
// what the user's umask gives any new file. Without --overwrite nothing that appeared at `localPath`
// meanwhile is replaced: a hard link fails on anything there, a dangling symlink included.
async function publishedMode(localPath: string, overwrite: boolean): Promise<number> {
  const existing = overwrite ? await statLocal(localPath) : null
  return existing ? existing.mode & 0o777 : 0o666 & ~process.umask()
}

async function publish(partialPath: string, localPath: string, overwrite: boolean): Promise<void> {
  if (overwrite) {
    await rename(partialPath, localPath)
    return
  }
  try { await link(partialPath, localPath) } catch (error) {
    if ((error as NodeJS.ErrnoException).code === "EEXIST") throw new CommandFailure("file_exists", `${localPath} appeared while the download ran; the verified download is kept as ${partialPath}. Pass --overwrite to publish it over ${localPath}`)
    throw error
  }
  await rm(partialPath)
}

// The staging name is predictable, so in a directory others can write to it may be a planted link or
// someone else's file. Resume only a private regular file of ours with an expected hash to verify it.
async function openStaging(partialPath: string, size: number, resume: boolean): Promise<{ handle: FileHandle; resumedFrom: number }> {
  const untrusted = (detail: string) => new CommandFailure("file_staging_untrusted", `${partialPath} ${detail}; remove it and run the same command again`)
  const mine = (info: Stats) => info.isFile() && info.uid === process.geteuid?.() && (info.mode & 0o077) === 0
  const existing = await lstat(partialPath).catch(error => {
    if ((error as NodeJS.ErrnoException).code === "ENOENT") return null
    throw error
  })
  if (resume && existing && mine(existing) && existing.size <= size) {
    const handle = await open(partialPath, constants.O_RDWR | constants.O_APPEND | constants.O_NOFOLLOW)
    const opened = await handle.stat()
    if (mine(opened) && opened.ino === existing.ino && opened.dev === existing.dev) return { handle, resumedFrom: opened.size }
    await handle.close()
    throw untrusted("changed while it was being opened")
  }
  if (existing) await rm(partialPath).catch(() => { throw untrusted("is not a private file of yours and could not be removed") })
  const handle = await open(partialPath, constants.O_RDWR | constants.O_CREAT | constants.O_EXCL | constants.O_NOFOLLOW, 0o600).catch(() => { throw untrusted("appeared while the download started") })
  return { handle, resumedFrom: 0 }
}

async function hashOf(handle: FileHandle): Promise<string> {
  const hash = createHash("sha256")
  for await (const chunk of createReadStream("", { fd: handle.fd, start: 0, autoClose: false })) hash.update(chunk)
  return hash.digest("hex")
}

async function getFile(send: SendRequest, remotePath: string, localPath: string, overwrite: boolean, expectedSha256: string | null): Promise<Transfer> {
  const started = Date.now()
  if (!overwrite && await statLocal(localPath)) throw new CommandFailure("file_exists", `${localPath} already exists; pass --overwrite to replace it`)
  const { size } = await readRemote(send, remotePath, 0, 0)
  const partialPath = localPath + PARTIAL_SUFFIX
  // Private while the bytes arrive; the published file gets its permissions below.
  const { handle: partial, resumedFrom } = await openStaging(partialPath, size, expectedSha256 !== null)
  let sha256: string
  try {
    for (let offset = resumedFrom; offset < size;) {
      const chunk = await readRemote(send, remotePath, offset, Math.min(CHUNK_BYTES, size - offset))
      const bytes = Buffer.from(chunk.data_base64, "base64")
      if (chunk.size !== size) throw new CommandFailure("file_io_failed", `${remotePath} changed from ${size} to ${chunk.size} bytes while it was downloaded; remove ${partialPath} and run the same command again`)
      if (bytes.length === 0) throw new CommandFailure("file_io_failed", `${remotePath} ended at ${offset} of ${size} bytes; run the same command again to retry`)
      await partial.writeFile(bytes)
      offset += bytes.length
    }
    sha256 = await hashOf(partial)
    if (expectedSha256 !== null && sha256 !== expectedSha256) {
      await rm(partialPath)
      throw new CommandFailure("file_hash_mismatch", `The download has sha256 ${sha256}, not the expected ${expectedSha256}, so nothing was kept; run the same command again to download it afresh`)
    }
    // Permissions are set on the file we hold, and what is published must still be that file.
    await partial.chmod(await publishedMode(localPath, overwrite))
    const published = await lstat(partialPath)
    const held = await partial.stat()
    if (published.ino !== held.ino || published.dev !== held.dev) throw new CommandFailure("file_staging_untrusted", `${partialPath} was replaced during the download; remove it and run the same command again`)
  } finally { await partial.close() }
  await publish(partialPath, localPath, overwrite)
  return { path: localPath, size, sha256, bytes_sent_or_received: size - resumedFrom, resumed_from: resumedFrom, elapsed_ms: Date.now() - started }
}

async function putFile(send: SendRequest, localPath: string, remotePath: string, overwrite: boolean): Promise<Transfer> {
  const started = Date.now()
  const local = await statLocal(localPath)
  if (!local) throw new CommandFailure("file_not_found", `${localPath} does not exist on this computer`)
  if (!local.isFile()) throw new CommandFailure("file_not_a_file", `${localPath} is not a regular file`)
  const { size } = local
  const sha256 = await sha256Of(localPath)
  const partialSize = await remotePartialSize(send, remotePath)
  // The partial's bytes are not verified here: the target checks the whole file's sha256 and keeps nothing on a mismatch.
  const resumedFrom = partialSize <= size ? partialSize : 0
  const source = await open(localPath, "r")
  const buffer = Buffer.alloc(CHUNK_BYTES)
  let written!: { path: string; complete: boolean }
  try {
    let offset = resumedFrom
    do {
      const { bytesRead } = await source.read(buffer, 0, CHUNK_BYTES, offset)
      if (bytesRead === 0 && offset < size) throw new CommandFailure("file_io_failed", `${localPath} changed while it was being sent; run the same command again`)
      const response = await send({ WriteKernelFile: { path: remotePath, offset, data_base64: buffer.subarray(0, bytesRead).toString("base64"), total_size: size, sha256, overwrite } }) as { KernelFileWritten: typeof written }
      written = response.KernelFileWritten
      offset += bytesRead
    } while (offset < size)
  } finally { await source.close() }
  if (!written.complete) throw new CommandFailure("file_io_failed", `${remotePath} was not completed by the last chunk; run the same command again to continue`)
  return { path: written.path, size, sha256, bytes_sent_or_received: size - resumedFrom, resumed_from: resumedFrom, elapsed_ms: Date.now() - started }
}

export async function runFilesVerb(
  verb: FilesVerb,
  [first, second]: readonly string[],
  flags: { sha256: string | null; overwrite: boolean },
  send: SendRequest,
): Promise<unknown> {
  switch (verb) {
    // The launcher runs from the repository; a relative local path means the agent's directory.
    case "get": return getFile(send, first!, resolve(terminalCwd(), second!), flags.overwrite, flags.sha256)
    case "put": return putFile(send, resolve(terminalCwd(), first!), second!, flags.overwrite)
  }
}

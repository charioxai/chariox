import { createHash } from "node:crypto"
import { constants, createReadStream, type Stats } from "node:fs"
import { resolve } from "node:path"
import { link, lstat, open, rename, rm, stat, type FileHandle } from "node:fs/promises"
import { CommandFailure } from "./agent-command-table.js"
import { terminalCwd } from "./app-install-file.js"

// Matches the kernel: a chunk travels base64 inside one relayed request of at most 768 KiB.
const CHUNK_BYTES = 256 * 1024
// The same name the kernel gives an unfinished upload, so a download resumes the way an upload does.
const PARTIAL_SUFFIX = ".chariox-partial"

export const FILES_VERBS = ["stat", "ls", "get", "put"] as const
export type FilesVerb = typeof FILES_VERBS[number]
export type SendRequest = (body: Record<string, unknown>) => Promise<unknown>

type RemoteStat = { path: string; exists: boolean; kind: string | null; size: number | null; sha256: string | null; partial_size: number | null }
type Transfer = { path: string; size: number; sha256: string; bytes_sent_or_received: number; resumed_from: number; elapsed_ms: number }

async function remoteStat(send: SendRequest, path: string, withSha256: boolean): Promise<RemoteStat> {
  const response = await send({ StatKernelFile: { path, with_sha256: withSha256 } }) as { KernelFileStat: { stat: RemoteStat } }
  return response.KernelFileStat.stat
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
// someone else's file. Resume only a private regular file of ours; anything else is replaced.
async function openStaging(partialPath: string, size: number): Promise<{ handle: FileHandle; resumedFrom: number }> {
  const untrusted = (detail: string) => new CommandFailure("file_staging_untrusted", `${partialPath} ${detail}; remove it and run the same command again`)
  const mine = (info: Stats) => info.isFile() && info.uid === process.geteuid?.() && (info.mode & 0o077) === 0
  const existing = await lstat(partialPath).catch(error => {
    if ((error as NodeJS.ErrnoException).code === "ENOENT") return null
    throw error
  })
  if (existing && mine(existing) && existing.size <= size) {
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

async function getFile(send: SendRequest, remotePath: string, localPath: string, overwrite: boolean): Promise<Transfer> {
  const started = Date.now()
  if (!overwrite && await statLocal(localPath)) throw new CommandFailure("file_exists", `${localPath} already exists; pass --overwrite to replace it`)
  const source = await remoteStat(send, remotePath, true)
  if (!source.exists) throw new CommandFailure("file_not_found", `${remotePath} does not exist on that kernel`)
  if (source.kind !== "file" || source.size === null || source.sha256 === null) throw new CommandFailure("file_not_a_file", `${remotePath} is not a regular file; use files ls for a directory`)
  const { size, sha256 } = source
  const partialPath = localPath + PARTIAL_SUFFIX
  // Private while the bytes arrive; the published file gets its permissions below.
  const { handle: partial, resumedFrom } = await openStaging(partialPath, size)
  try {
    for (let offset = resumedFrom; offset < size;) {
      const response = await send({ ReadKernelFile: { path: remotePath, offset, length: Math.min(CHUNK_BYTES, size - offset) } }) as { KernelFileChunk: { data_base64: string } }
      const bytes = Buffer.from(response.KernelFileChunk.data_base64, "base64")
      if (bytes.length === 0) throw new CommandFailure("file_io_failed", `${remotePath} ended at ${offset} of ${size} bytes; run the same command again to continue`)
      await partial.writeFile(bytes)
      offset += bytes.length
    }
    if (await hashOf(partial) !== sha256) {
      await rm(partialPath)
      throw new CommandFailure("file_hash_mismatch", `The download did not match the sha256 of ${remotePath}, so nothing was kept; run the same command again to download it afresh`)
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
  const target = await remoteStat(send, remotePath, false)
  // The partial's bytes are not verified here: the target checks the whole file's sha256 and keeps nothing on a mismatch.
  const resumedFrom = target.partial_size !== null && target.partial_size <= size ? target.partial_size : 0
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
  flags: { sha256: boolean; overwrite: boolean },
  send: SendRequest,
): Promise<unknown> {
  switch (verb) {
    case "stat": return remoteStat(send, first!, flags.sha256)
    case "ls": return (await send({ ListKernelDirectory: { path: first! } }) as { KernelDirectoryListed: unknown }).KernelDirectoryListed
    // The launcher runs from the repository; a relative local path means the agent's directory.
    case "get": return getFile(send, first!, resolve(terminalCwd(), second!), flags.overwrite)
    case "put": return putFile(send, resolve(terminalCwd(), first!), second!, flags.overwrite)
  }
}

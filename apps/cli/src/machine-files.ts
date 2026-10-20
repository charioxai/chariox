import { createHash } from "node:crypto"
import { createReadStream, type Stats } from "node:fs"
import { open, rename, rm, stat } from "node:fs/promises"
import { CommandFailure } from "./agent-command-table.js"

const CHUNK_BYTES = 1024 * 1024
// The same name the kernel gives an unfinished upload, so a download resumes the way an upload does.
const PARTIAL_SUFFIX = ".chariox-partial"

export const FILES_VERBS = ["stat", "ls", "get", "put"] as const
export type FilesVerb = typeof FILES_VERBS[number]
export type SendRequest = (body: Record<string, unknown>) => Promise<unknown>

type RemoteStat = { path: string; exists: boolean; kind: string | null; size: number | null; sha256: string | null; partial_size: number | null }
type Transfer = { path: string; size: number; sha256: string; bytes_sent_or_received: number; resumed_from: number; elapsed_ms: number }

async function remoteStat(send: SendRequest, path: string, withSha256: boolean): Promise<RemoteStat> {
  const response = await send({ StatMachineFile: { path, with_sha256: withSha256 } }) as { MachineFileStat: { stat: RemoteStat } }
  return response.MachineFileStat.stat
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

async function getFile(send: SendRequest, remotePath: string, localPath: string, overwrite: boolean): Promise<Transfer> {
  const started = Date.now()
  if (!overwrite && await statLocal(localPath)) throw new CommandFailure("file_exists", `${localPath} already exists; pass --overwrite to replace it`)
  const source = await remoteStat(send, remotePath, true)
  if (!source.exists) throw new CommandFailure("file_not_found", `${remotePath} does not exist on that machine`)
  if (source.kind !== "file" || source.size === null || source.sha256 === null) throw new CommandFailure("file_not_a_file", `${remotePath} is not a regular file; use files ls for a directory`)
  const { size, sha256 } = source
  const partialPath = localPath + PARTIAL_SUFFIX
  const have = (await statLocal(partialPath))?.size ?? 0
  const resumedFrom = have <= size ? have : 0
  const partial = await open(partialPath, resumedFrom > 0 ? "a" : "w")
  try {
    for (let offset = resumedFrom; offset < size;) {
      const response = await send({ ReadMachineFile: { path: remotePath, offset, length: Math.min(CHUNK_BYTES, size - offset) } }) as { MachineFileChunk: { data_base64: string } }
      const bytes = Buffer.from(response.MachineFileChunk.data_base64, "base64")
      if (bytes.length === 0) throw new CommandFailure("file_io_failed", `${remotePath} ended at ${offset} of ${size} bytes; run the same command again to continue`)
      await partial.writeFile(bytes)
      offset += bytes.length
    }
  } finally { await partial.close() }
  if (await sha256Of(partialPath) !== sha256) {
    await rm(partialPath)
    throw new CommandFailure("file_hash_mismatch", `The download did not match the sha256 of ${remotePath}, so nothing was kept; run the same command again to download it afresh`)
  }
  await rename(partialPath, localPath)
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
      const response = await send({ WriteMachineFile: { path: remotePath, offset, data_base64: buffer.subarray(0, bytesRead).toString("base64"), total_size: size, sha256, overwrite } }) as { MachineFileWritten: typeof written }
      written = response.MachineFileWritten
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
    case "ls": return (await send({ ListMachineDirectory: { path: first! } }) as { MachineDirectoryListed: unknown }).MachineDirectoryListed
    case "get": return getFile(send, first!, second!, flags.overwrite)
    case "put": return putFile(send, first!, second!, flags.overwrite)
  }
}

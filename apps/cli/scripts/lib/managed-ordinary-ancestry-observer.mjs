// MP-01/MP-10: executable ancestry is observation, never caller booleans or
// command-line substring matching. Do not read or emit process arguments.
import { createHash } from "node:crypto"
import { readFile, readlink } from "node:fs/promises"
import { basename, isAbsolute } from "node:path"
import { inspectPrivilegedProcMetadata } from "./managed-ordinary-proc-metadata.mjs"

function identity(pid, text) {
  const close = text.lastIndexOf(")")
  const fields = text.slice(close + 2).trim().split(/\s+/)
  if (text.slice(0, text.indexOf("(")).trim() !== String(pid) || close < 0
    || ["Z", "X"].includes(fields[0]) || !/^[0-9]+$/.test(fields[19] ?? "")) {
    throw new Error("MP-01 invalid ancestry process identity")
  }
  const parentPid = Number(fields[1])
  if (!Number.isSafeInteger(parentPid) || parentPid < 0) throw new Error("MP-01 invalid ancestry parent")
  return { parentPid, startTimeTicks: fields[19] }
}

export async function observeExecutableAncestry({
  pid = process.pid, filesystem = { readFile, readlink }, privilegedMetadata = inspectPrivilegedProcMetadata,
} = {}) {
  const chain = []
  const seen = new Set()
  while (chain.length < 128) {
    if (!Number.isSafeInteger(pid) || pid < 1 || seen.has(pid)) throw new Error("MP-01 incomplete or cyclic ancestry")
    seen.add(pid)
    const path = `/proc/${pid}`
    const before = identity(pid, await filesystem.readFile(`${path}/stat`, "utf8"))
    let executable
    try { executable = await filesystem.readlink(`${path}/exe`) } catch (error) {
      if (!["EACCES", "EPERM"].includes(error.code)) throw error
      const metadata = await privilegedMetadata("process", pid)
      if (metadata.pid !== pid || identity(pid, metadata.stat).startTimeTicks !== before.startTimeTicks) {
        throw new Error("MP-01 ancestry changed during privileged inspection")
      }
      executable = metadata.executableLink
    }
    const after = identity(pid, await filesystem.readFile(`${path}/stat`, "utf8"))
    if (before.parentPid !== after.parentPid || before.startTimeTicks !== after.startTimeTicks
      || !isAbsolute(executable) || executable.endsWith(" (deleted)")) {
      throw new Error("MP-01 ancestry identity changed or executable unavailable")
    }
    chain.push({ pid, ...after, executable_basename: basename(executable),
      executable_path_sha256: `sha256:${createHash("sha256").update(executable).digest("hex")}` })
    if (pid === 1) {
      for (const entry of chain) {
        const retained = identity(entry.pid, await filesystem.readFile(`/proc/${entry.pid}/stat`, "utf8"))
        if (retained.parentPid !== entry.parentPid || retained.startTimeTicks !== entry.startTimeTicks) {
          throw new Error("MP-01 ancestry changed before capture completed")
        }
      }
      return chain
    }
    pid = after.parentPid
  }
  throw new Error("MP-01 ancestry exceeds depth bound")
}

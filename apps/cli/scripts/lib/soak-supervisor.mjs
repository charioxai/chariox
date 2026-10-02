// MP-08 / MP-10: an explicitly identified container supervisor belongs to the
// measured run, but survives each phase's runtime cleanup.
import { readFile, readlink } from "node:fs/promises"

export async function verifiedSoakSupervisor({
  identity = process.env.CHARIOX_SOAK_SUPERVISOR_IDENTITY,
  read = readFile,
  link = readlink,
  currentPid = process.pid,
} = {}) {
  if (!identity) return null
  const expected = JSON.parse(identity)
  if (expected.pid !== 1 || !/^\d+$/.test(expected.startedAtTicks ?? "")) {
    throw new Error("soak supervisor requires an exact container PID 1 identity")
  }
  const fields = (await read("/proc/1/stat", "utf8")).split(")").at(-1).trim().split(/\s+/)
  const executable = await link("/proc/1/exe")
  const argv = await read("/proc/1/cmdline", "utf8")
  if (fields[0] === "Z" || fields[19] !== expected.startedAtTicks
    || executable !== "/usr/local/bin/node" || executable !== expected.executable
    || Number(fields[2]) !== expected.processGroupId
    || argv !== "/usr/local/bin/node\0/pilot/supervisor.mjs\0") {
    throw new Error("soak supervisor process identity changed")
  }
  for (const namespace of ["pid", "net"]) {
    if (await link(`/proc/1/ns/${namespace}`) !== await link(`/proc/${currentPid}/ns/${namespace}`)) {
      throw new Error("soak supervisor is outside the runner namespace")
    }
  }
  if (await read("/proc/1/cgroup", "utf8") !== await read(`/proc/${currentPid}/cgroup`, "utf8")) {
    throw new Error("soak supervisor is outside the runner cgroup")
  }
  return { ...expected, scope: "supervisor" }
}

export function includeSoakSupervisor(rows, ownedIds, supervisor) {
  const ids = new Set(ownedIds)
  if (supervisor) ids.add(supervisor.pid)
  return rows.filter(row => ids.has(row.pid)).map(row => ({
    ...row, ...(row.pid === supervisor?.pid ? { scope: "supervisor" } : {}),
  }))
}

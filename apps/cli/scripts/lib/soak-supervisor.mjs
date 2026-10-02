// MP-08 / MP-10: measure the identified supervisor and its init reaper.
import { readFile, readlink } from "node:fs/promises"

export async function verifiedSoakSupervisor({
  identity = process.env.CHARIOX_SOAK_SUPERVISOR_IDENTITY,
  read = readFile,
  link = readlink,
  currentPid = process.pid,
} = {}) {
  if (!identity) return null
  const expected = JSON.parse(identity)
  if (expected.reaper?.pid !== 1 || !Number.isSafeInteger(expected.supervisor?.pid) || expected.supervisor.pid <= 1) {
    throw new Error("soak supervisor requires its exact container PID 1 reaper identity")
  }
  for (const [kind, executable, argv] of [
    ["reaper", "/usr/sbin/docker-init", "/sbin/docker-init\0--\0/usr/local/bin/node\0/pilot/supervisor.mjs\0"],
    ["supervisor", "/usr/local/bin/node", "/usr/local/bin/node\0/pilot/supervisor.mjs\0"],
  ]) {
    const value = expected[kind]
    const fields = (await read(`/proc/${value.pid}/stat`, "utf8")).split(")").at(-1).trim().split(/\s+/)
    if (!/^\d+$/.test(value.startedAtTicks ?? "") || fields[0] === "Z" || fields[19] !== value.startedAtTicks
      || await link(`/proc/${value.pid}/exe`) !== executable || executable !== value.executable
      || Number(fields[2]) !== value.processGroupId || (kind === "supervisor" && Number(fields[1]) !== 1)
      || await read(`/proc/${value.pid}/cmdline`, "utf8") !== argv) {
      throw new Error("soak supervisor process identity changed")
    }
    for (const namespace of ["pid", "net"]) {
      if (await link(`/proc/${value.pid}/ns/${namespace}`) !== await link(`/proc/${currentPid}/ns/${namespace}`)) {
        throw new Error("soak supervisor is outside the runner namespace")
      }
    }
    if (await read(`/proc/${value.pid}/cgroup`, "utf8") !== await read(`/proc/${currentPid}/cgroup`, "utf8")) {
      throw new Error("soak supervisor is outside the runner cgroup")
    }
  }
  return expected
}

export function includeSoakSupervisor(rows, ownedIds, supervisor) {
  const ids = new Set(ownedIds)
  const infrastructure = new Set(supervisor ? [supervisor.reaper.pid, supervisor.supervisor.pid] : [])
  for (const pid of infrastructure) ids.add(pid)
  let changed = true
  while (changed) {
    changed = false
    for (const row of rows) if (ids.has(row.ppid) && !ids.has(row.pid)) { ids.add(row.pid); changed = true }
  }
  return rows.filter(row => ids.has(row.pid)).map(row => ({
    ...row, ...(infrastructure.has(row.pid) ? { scope: "supervisor" } : {}),
  }))
}

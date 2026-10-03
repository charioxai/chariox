function fail(message) {
  throw new Error(message)
}

function quotaProjectId(projectId) {
  if (!Number.isInteger(projectId) || projectId < 0 || projectId > 0xffff_ffff) {
    fail("XFS project quota ID is malformed")
  }
  return String(projectId)
}

function reportRows(output) {
  if (typeof output !== "string") fail("XFS project quota readback is malformed")
  const text = output.trim()
  if (!text) return []

  return text.split(/\r?\n/).map((line) => {
    const match = /^#(0|[1-9][0-9]*)\s+(0|[1-9][0-9]*)\s+(0|[1-9][0-9]*)\s+(0|[1-9][0-9]*)\s+([0-9]{2}|[1-9][0-9]{2,})\s+(\[[^\[\]\r\n]+\])$/.exec(line.trim())
    if (!match || !/^\[(?:--none--|--------|[0-9]+ days?|[0-9]{2}:[0-9]{2}:[0-9]{2})\]$/.test(match[6])) {
      fail("XFS project quota readback is malformed")
    }

    const projectId = BigInt(match[1])
    const usedKiB = BigInt(match[2])
    const softLimitKiB = BigInt(match[3])
    const hardLimitKiB = BigInt(match[4])
    const warnings = BigInt(match[5])
    const maxUint32 = 0xffff_ffffn
    const maxUint64 = 0xffff_ffff_ffff_ffffn
    if (
      projectId > maxUint32 ||
      usedKiB > maxUint64 ||
      softLimitKiB > maxUint64 ||
      hardLimitKiB > maxUint64 ||
      warnings > 0xffffn
    ) {
      fail("XFS project quota readback exceeds the supported range")
    }

    return {
      projectId: Number(projectId),
      usedKiB,
      softLimitKiB,
      hardLimitKiB,
    }
  })
}

function kibibytesToSafeBytes(value) {
  const bytes = value * 1024n
  if (bytes > BigInt(Number.MAX_SAFE_INTEGER)) fail("XFS quota readback exceeds the safe integer range")
  return Number(bytes)
}

export function parseProjectQuotaState(output) {
  if (typeof output !== "string") fail("XFS project quota state is malformed")
  const lines = output.split(/\r?\n/)
  const projectHeadings = []
  for (let index = 0; index < lines.length; index += 1) {
    if (/Project quota state/.test(lines[index])) projectHeadings.push(index)
  }
  if (projectHeadings.length !== 1) fail("XFS project quota state is missing or duplicated")

  const headingIndex = projectHeadings[0]
  if (!/^Project quota state on .+ \(.+\)$/.test(lines[headingIndex].trim())) {
    fail("XFS project quota state heading is malformed")
  }

  const stateLines = []
  for (let index = headingIndex + 1; index < lines.length; index += 1) {
    if (/^(?:User|Group|Project) quota state on /.test(lines[index].trim())) break
    stateLines.push(lines[index].trim())
  }

  const accounting = []
  const enforcement = []
  for (const line of stateLines) {
    if (/^Accounting\b/.test(line)) {
      const match = /^Accounting: (ON|OFF)$/.exec(line)
      if (!match) fail("XFS project quota accounting state is malformed")
      accounting.push(match[1] === "ON")
    }
    if (/^Enforcement\b/.test(line)) {
      const match = /^Enforcement: (ON|OFF)$/.exec(line)
      if (!match) fail("XFS project quota enforcement state is malformed")
      enforcement.push(match[1] === "ON")
    }
  }
  if (accounting.length !== 1 || enforcement.length !== 1) {
    fail("XFS project quota accounting or enforcement state is missing or duplicated")
  }

  return { accounting: accounting[0], enforcement: enforcement[0] }
}

export function checkProjectQuotaTree(runXfsQuota, mountpoint, path, projectId) {
  if (typeof runXfsQuota !== "function") fail("XFS project quota command runner is unavailable")
  const id = quotaProjectId(projectId)
  if (typeof path !== "string" || !/^\/[A-Za-z0-9_./-]+$/.test(path)) {
    fail("XFS project quota tree path is malformed")
  }

  const output = runXfsQuota(mountpoint, `project -c -p ${path} ${id}`)
  if (typeof output !== "string") fail("XFS project quota tree readback is malformed")
  const text = output.trim()
  const lines = text ? text.split(/\r?\n/) : []
  const checking = `Checking project ${id} (path ${path})...`
  const processed = `Processed 1 (/dev/null and cmdline) paths for project ${id} with recursion depth infinite (-1).`
  if (lines.length < 2 || lines[0].trim() !== checking || lines.at(-1).trim() !== processed) {
    fail("XFS project quota tree readback is malformed")
  }

  const seenMismatches = new Set()
  for (const line of lines.slice(1, -1)) {
    const text = line.trim()
    const idMismatch = /^(.*) - project identifier is not set \(inode=([0-9]+), tree=([0-9]+)\)$/.exec(text)
    const inheritanceMismatch = /^(.*) - project inheritance flag is not set$/.exec(text)
    const mismatch = idMismatch ?? inheritanceMismatch
    if (!mismatch) fail("XFS project quota tree readback contains unexpected output")
    const itemPath = mismatch[1]
    if (itemPath !== path && !itemPath.startsWith(`${path}/`)) {
      fail("XFS project quota tree readback refers to a path outside the checked tree")
    }
    if (idMismatch && (Number(idMismatch[2]) === projectId || Number(idMismatch[3]) !== projectId)) {
      fail("XFS project quota tree readback contains an inconsistent project ID")
    }
    const key = `${itemPath}\0${idMismatch ? "id" : "inheritance"}`
    if (seenMismatches.has(key)) fail("XFS project quota tree readback contains duplicate mismatch output")
    seenMismatches.add(key)
  }
  return seenMismatches.size === 0
}

export function assertProjectQuotaTreeMatches(runXfsQuota, mountpoint, path, projectId) {
  if (!checkProjectQuotaTree(runXfsQuota, mountpoint, path, projectId)) {
    fail("XFS project quota tree contains a mismatched project assignment")
  }
  return true
}

export function readProjectQuotaRow(runXfsQuota, mountpoint, projectId, { required = false } = {}) {
  if (typeof runXfsQuota !== "function") fail("XFS project quota command runner is unavailable")
  const id = quotaProjectId(projectId)
  const output = runXfsQuota(mountpoint, `report -p -b -n -N -L ${id} -U ${id}`)
  const rows = reportRows(output)
  if (rows.length === 0) {
    if (required) fail("XFS project quota readback is missing")
    return { projectId, found: false, usedBytes: 0, hardLimitBytes: 0 }
  }
  if (rows.length !== 1) fail("XFS project quota readback contains duplicate rows")
  const row = rows[0]
  if (row.projectId !== projectId) fail("XFS project quota readback returned a different project ID")

  return {
    projectId,
    found: true,
    usedBytes: kibibytesToSafeBytes(row.usedKiB),
    hardLimitBytes: kibibytesToSafeBytes(row.hardLimitKiB),
  }
}

export function readProjectQuotaIds(runXfsQuota, mountpoint) {
  if (typeof runXfsQuota !== "function") fail("XFS project quota command runner is unavailable")
  const rows = reportRows(runXfsQuota(mountpoint, "report -p -b -n -N"))
  const ids = []
  const seen = new Set()
  for (const row of rows) {
    if (seen.has(row.projectId)) fail("XFS project quota report contains duplicate project IDs")
    seen.add(row.projectId)
    ids.push(row.projectId)
  }
  return ids
}

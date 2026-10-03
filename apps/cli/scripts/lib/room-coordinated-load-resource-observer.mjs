import { execFile } from "node:child_process"
import { promisify } from "node:util"

const execFileAsync = promisify(execFile)
const processCommandTimeoutMs = 10_000
const maximumCommandOutputBytes = 2 * 1024 * 1024

export async function readDockerInventory(plan, options = {}) {
  const run = options.execFileAsync ?? execFileAsync
  const { stdout: containerText } = await run("docker", ["ps", "--all", "--no-trunc", "--format", "{{json .}}"], {
    timeout: processCommandTimeoutMs,
    maxBuffer: maximumCommandOutputBytes,
  })
  const containers = containerText.split("\n").filter(Boolean).map((line) => {
    const row = JSON.parse(line)
    return {
      id: safeString(row.ID),
      name: safeString(row.Names),
      status: safeString(row.Status).split(/\s+/, 1)[0],
      ports: safeString(row.Ports),
    }
  })
  const { stdout: volumeText } = await run("docker", ["volume", "ls", "--format", "{{.Name}}"], {
    timeout: processCommandTimeoutMs,
    maxBuffer: maximumCommandOutputBytes,
  })
  const volumes = volumeText.split("\n").map((name) => name.trim()).filter(Boolean)
  const expected = new Set(plan.headedSlices.map((slice) => `chariox-slice-${slice.sliceName}`))
  for (const name of expected) {
    if (containers.filter((container) => container.name === name).length !== 1) {
      throw new Error("exact prepared slice container is unavailable or ambiguous")
    }
  }
  return { containers, volumes }
}

export async function readDockerStats(plan, options = {}) {
  const run = options.execFileAsync ?? execFileAsync
  const names = plan.headedSlices.map((slice) => `chariox-slice-${slice.sliceName}`)
  const { stdout } = await run("docker", [
    "stats", "--no-stream", "--format", "{{.Name}}|{{.CPUPerc}}|{{.MemUsage}}", ...names,
  ], { timeout: processCommandTimeoutMs, maxBuffer: maximumCommandOutputBytes })
  const rows = stdout.split("\n").filter(Boolean).map((line) => {
    const [name, cpu, memory] = line.split("|")
    const [used] = memory?.split("/") ?? []
    return { name, cpuPercent: parsePercent(cpu), memoryBytes: parseByteCount(used) }
  })
  if (rows.length !== names.length || new Set(rows.map((row) => row.name)).size !== names.length
    || rows.some((row) => !names.includes(row.name))) {
    throw new Error("Docker did not return exact stats for every prepared headed slice")
  }
  return {
    cpuPercent: rows.reduce((total, row) => total + row.cpuPercent, 0),
    memoryBytes: rows.reduce((total, row) => total + row.memoryBytes, 0),
  }
}

export async function readListeners(ports, options = {}) {
  const run = options.execFileAsync ?? execFileAsync
  const args = ["-nP", ...ports.flatMap((port) => [`-iTCP:${port}`]), "-sTCP:LISTEN", "-Fpn"]
  const { stdout } = await run("lsof", args, {
    timeout: processCommandTimeoutMs,
    maxBuffer: maximumCommandOutputBytes,
  })
  const exactPorts = new Set(ports)
  let pid = null
  const found = new Set()
  for (const line of stdout.split("\n")) {
    if (line.startsWith("p")) pid = line.slice(1)
    if (!line.startsWith("n") || !pid) continue
    const match = line.match(/:(\d+)(?:\s|$)/)
    const port = match ? Number(match[1]) : null
    if (port !== null && exactPorts.has(port)) found.add(`${port}:${pid}`)
  }
  return [...found].sort()
}

export async function countListeners(ports, options = {}) {
  return (await readListeners(ports, options)).length
}

function parseByteCount(value) {
  const match = String(value ?? "").trim().match(/^([0-9]+(?:\.[0-9]+)?)\s*(B|kB|KB|KiB|MB|MiB|GB|GiB|TB|TiB)$/i)
  if (!match) throw new Error("Docker returned an unsupported memory unit")
  const factors = { b: 1, kb: 1_000, kib: 1024, mb: 1_000_000, mib: 1024 ** 2, gb: 1_000_000_000, gib: 1024 ** 3, tb: 1_000_000_000_000, tib: 1024 ** 4 }
  return Math.round(Number(match[1]) * factors[match[2].toLowerCase()])
}

function parsePercent(value) {
  const parsed = Number(String(value ?? "").replace("%", ""))
  if (!Number.isFinite(parsed) || parsed < 0) throw new Error("Docker returned an invalid CPU sample")
  return parsed
}

function safeString(value) {
  return typeof value === "string" ? value : ""
}

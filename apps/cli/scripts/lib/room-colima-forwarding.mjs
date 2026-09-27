import { spawn } from "node:child_process"
import path from "node:path"
import { setTimeout as sleep } from "node:timers/promises"

export async function startRoomSliceWithForwarding({ sshConfig, slice, startSlice, containerExists }, dependencies = {}) {
  if (!sshConfig) { await startSlice(); return null }
  // Validate before provisioning creates resources, but do not bind yet:
  // the kernel checks host port availability before creating the container.
  if (!path.isAbsolute(sshConfig)) throw new Error("Colima SSH config must be an absolute path")
  ownedPorts(slice)
  let startError
  const started = Promise.resolve().then(startSlice).catch((error) => { startError = error })
  let forward
  try {
    const deadline = Date.now() + 30000
    while (true) {
      if (startError) throw startError
      const exists = await containerExists()
      if (startError) throw startError
      if (exists) break
      if (Date.now() >= deadline) throw new Error("owned slice container did not appear for forwarding")
      await sleep(100)
    }
    forward = await startRoomColimaForwarding({ sshConfig, slice }, dependencies)
    await started
    if (startError) throw startError
    forward.assertHealthy()
    return forward
  } catch (error) {
    // A failed forward must not race cleanup against an in-flight provisioner.
    await started
    await forward?.close()
    throw error
  }
}

function ownedPorts(slice) {
  if (slice?.backend !== "local_docker" || !/^room-pointer-\d+-/.test(slice.name ?? "")
      || slice.owner_kernel_id !== `${slice.name}-home`) {
    throw new Error("Colima forwarding requires the drill-owned local slice")
  }
  const assigned = slice.local_docker_ports ?? {}
  const ports = new Set()
  const add = (port) => {
    if (!Number.isInteger(port) || port < 1024 || port > 65535) throw new Error("invalid owned slice port")
    ports.add(port)
  }
  for (const key of ["kernel", "relay", "novnc"]) add(assigned[key])
  for (const key of ["codex", "opencode"]) if (assigned[key] !== undefined) add(assigned[key])
  // The kernel's assigned provider ranges contain 20 ports each.
  for (const key of ["codex_range_start", "opencode_range_start"]) {
    if (assigned[key] !== undefined) for (let offset = 0; offset < 20; offset++) add(assigned[key] + offset)
  }
  return [...ports].sort((a, b) => a - b)
}

async function finishesWithin(promise, timeoutMs) {
  let timer
  try {
    return await Promise.race([promise.then(() => true), new Promise((resolve) => {
      timer = setTimeout(() => resolve(false), timeoutMs)
    })])
  } finally { clearTimeout(timer) }
}

export async function startRoomColimaForwarding({ sshConfig, slice } = {}, dependencies = {}) {
  if (!sshConfig) return null
  if (!path.isAbsolute(sshConfig)) throw new Error("Colima SSH config must be an absolute path")
  const ports = ownedPorts(slice)
  const args = ["-F", sshConfig, "-o", "ControlMaster=no", "-o", "ControlPath=none",
    "-o", "ControlPersist=no", "-o", "ExitOnForwardFailure=yes", "-o", "ConnectTimeout=5",
    "-o", "ServerAliveInterval=5", "-o", "ServerAliveCountMax=2", "-T", "-N", "-v"]
  for (const port of ports) args.push("-L", `127.0.0.1:${port}:127.0.0.1:${port}`)
  args.push("colima")
  const child = (dependencies.spawn ?? spawn)("ssh", args, { stdio: ["ignore", "ignore", "pipe"] })
  let exited = false
  let failed = false
  let closing
  const closed = new Promise((resolve) => child.once("close", () => { exited = true; resolve() }))
  const close = () => closing ??= (async () => {
    if (!exited && child.pid && child.exitCode === null && child.signalCode === null) child.kill("SIGTERM")
    if (await finishesWithin(closed, 1000)) return
    if (child.pid) child.kill("SIGKILL")
    if (!await finishesWithin(closed, 1000)) throw new Error("owned Colima forward did not exit")
  })()
  try {
    await new Promise((resolve, reject) => {
      const listening = new Set()
      let partial = ""
      const timer = setTimeout(() => finish(new Error("owned Colima forward startup timed out")), 10000)
      let settled = false
      const finish = (error) => {
        if (settled) return
        settled = true
        clearTimeout(timer)
        child.stderr.off("data", onData)
        // Drain SSH diagnostics without retaining config paths or host details.
        child.stderr.resume()
        error ? reject(error) : resolve()
      }
      const onData = (chunk) => {
        const lines = (partial + chunk.toString("utf8")).split("\n")
        partial = lines.pop().slice(-1024)
        for (const raw of lines) {
          const line = raw.endsWith("\r") ? raw.slice(0, -1) : raw
          const match = /^debug1: Local forwarding listening on 127\.0\.0\.1 port (\d+)\.$/.exec(line)
          if (match && ports.includes(Number(match[1]))) listening.add(Number(match[1]))
          if (line === "debug1: Entering interactive session." && listening.size === ports.length) finish()
        }
      }
      child.stderr.on("data", onData)
      child.once("error", () => { failed = true; finish(new Error("owned Colima forward could not start")) })
      child.once("exit", () => { failed = true; finish(new Error("owned Colima forward exited during startup")) })
    })
  } catch (error) {
    await close()
    throw error
  }
  return {
    ports,
    assertHealthy() {
      if (failed || exited || closing) throw new Error("owned Colima forward is not active")
    },
    close,
  }
}

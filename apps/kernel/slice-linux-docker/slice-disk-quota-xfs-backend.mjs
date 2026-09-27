import { spawnSync } from "node:child_process"
import { lstatSync, readFileSync, realpathSync, statSync, statfsSync } from "node:fs"
import { dirname, join, resolve } from "node:path"
import {
  SLICE_DISK_QUOTA_DATA_ROOT,
  SLICE_DISK_QUOTA_DOCKER_HOST,
} from "./slice-disk-quota-contract.mjs"

function fail(message) {
  throw new Error(message)
}

function command(commandPath, args, options = {}) {
  const result = spawnSync(commandPath, args, {
    env: { HOME: "/var/lib/chariox-docker/home", PATH: "/usr/bin:/usr/sbin:/bin:/sbin", DOCKER_HOST: SLICE_DISK_QUOTA_DOCKER_HOST },
    encoding: "utf8",
    maxBuffer: 1024 * 1024,
    timeout: 30_000,
    ...options,
  })
  if (result.error || result.status !== 0) {
    fail(`${commandPath.split("/").at(-1)} failed: ${(result.stderr ?? result.error?.message ?? "unknown error").trim()}`)
  }
  return (result.stdout ?? "").trim()
}

function dockerJson(args) {
  return JSON.parse(command("/usr/bin/docker", ["--host", SLICE_DISK_QUOTA_DOCKER_HOST, ...args]))
}

function runXfsQuota(mountpoint, expression) {
  return command("/usr/sbin/xfs_quota", ["-x", "-D", "/dev/null", "-P", "/dev/null", "-c", expression, mountpoint], {
    env: { PATH: "/usr/bin:/usr/sbin:/bin:/sbin" },
    timeout: 120_000,
  })
}

export function assertTrustedPath(targetPath, expectedPrefix, dockerUid) {
  if (typeof targetPath !== "string" || !targetPath.startsWith(`${expectedPrefix}/`) || /[^A-Za-z0-9_./-]/.test(targetPath)) {
    fail("Docker returned a writable-layer or volume path outside its fixed root")
  }
  const canonical = realpathSync(targetPath)
  if (canonical !== targetPath || !canonical.startsWith(`${expectedPrefix}/`)) fail("Docker storage path is not canonical")
  let prefix = "/"
  for (const component of canonical.slice(1).split("/")) {
    prefix = join(prefix, component)
    const info = lstatSync(prefix)
    if (info.isSymbolicLink()) fail("Docker storage path contains a symbolic link")
  }
  const info = statSync(canonical)
  if (!info.isDirectory() || info.uid !== dockerUid) fail("Docker storage path ownership is not the dedicated rootless engine")
  return canonical
}

function readServiceUid(name) {
  const row = readFileSync("/etc/passwd", "utf8").split("\n").find((line) => line.startsWith(`${name}:`))
  const uid = row?.split(":")[2]
  if (!/^[1-9][0-9]*$/.test(uid ?? "")) fail("dedicated rootless Docker account is unavailable")
  return Number(uid)
}

function parseQuotaRow(output, projectId) {
  const line = output.split("\n").map((row) => row.trim()).find((row) => row.startsWith(`${projectId} `) || row === String(projectId))
  if (!line) return { projectId, found: false, usedBytes: 0, hardLimitBytes: 0 }
  const fields = line.split(/\s+/)
  if (fields.length < 4 || fields[0] !== String(projectId)) fail("XFS project quota readback is malformed")
  const usedKiB = Number(fields[1])
  const hardKiB = Number(fields[3])
  if (!Number.isSafeInteger(usedKiB) || usedKiB < 0 || !Number.isSafeInteger(hardKiB) || hardKiB < 0) {
    fail("XFS project quota usage or limit is malformed")
  }
  const usedBytes = usedKiB * 1024
  const hardLimitBytes = hardKiB * 1024
  if (!Number.isSafeInteger(usedBytes) || !Number.isSafeInteger(hardLimitBytes)) fail("XFS quota readback exceeds the safe integer range")
  return { projectId, found: true, usedBytes, hardLimitBytes }
}

export function createSystemSliceDiskQuotaBackend({
  dataRoot = SLICE_DISK_QUOTA_DATA_ROOT,
  dockerUid = readServiceUid("chariox-docker"),
} = {}) {
  const canonicalDataRoot = resolve(dataRoot)

  function probe() {
    try {
      const info = dockerJson(["info", "--format", "{{json .}}"])
      if (info.Driver !== "overlay2") return { supported: false, reason: "managed quota mode requires Docker classic overlay2; containerd overlayfs snapshotter is unsupported" }
    if (info.DockerRootDir !== canonicalDataRoot) return { supported: false, reason: "DockerRootDir does not match the pinned managed quota data root" }
      const mount = JSON.parse(command("/usr/bin/findmnt", ["--json", "--target", canonicalDataRoot, "--output", "TARGET,FSTYPE,OPTIONS,SOURCE"], {
        env: { PATH: "/usr/bin:/bin" },
      })).filesystems?.[0]
      if (!mount || mount.target !== canonicalDataRoot || mount.fstype !== "xfs" || !["pquota", "prjquota"].some((option) => mount.options.split(",").includes(option))) {
        return { supported: false, reason: "managed DockerRootDir must be a dedicated XFS mount with project-quota enforcement" }
      }
      const state = runXfsQuota(mount.target, "state -p")
      const lines = state.split(/\r?\n/)
      const projectStateIndex = lines.findIndex((line) => line.includes("Project quota state"))
      if (projectStateIndex < 0) return { supported: false, reason: "XFS project-quota state could not be read" }
      const projectState = lines.slice(projectStateIndex).join("\n")
      if (!projectState.includes("Accounting: ON") || !projectState.includes("Enforcement: ON")) {
        return { supported: false, reason: "XFS project quota accounting and enforcement are not both active" }
      }
      const xfsInfo = command("/usr/sbin/xfs_info", [mount.target], { env: { PATH: "/usr/bin:/usr/sbin:/bin:/sbin" } })
      if (!/\bftype=1\b/.test(xfsInfo)) {
        return { supported: false, reason: "managed overlay2 backing XFS filesystem does not report ftype=1" }
      }
      const filesystem = statfsSync(canonicalDataRoot, { bigint: true })
      const totalBytes = Number(filesystem.blocks * filesystem.bsize)
      const availableBytes = Number(filesystem.bavail * filesystem.bsize)
      const hostFilesystem = statfsSync("/", { bigint: true })
      const hostTotalBytes = Number(hostFilesystem.blocks * hostFilesystem.bsize)
      const hostAvailableBytes = Number(hostFilesystem.bavail * hostFilesystem.bsize)
      if (![totalBytes, availableBytes, hostTotalBytes, hostAvailableBytes].every(Number.isSafeInteger)) {
        return { supported: false, reason: "managed quota filesystem or host capacity readback exceeds safe integer range" }
      }
      return { supported: true, driver: info.Driver, dockerRootDir: info.DockerRootDir, filesystem: mount.fstype, mountOptions: mount.options, mountpoint: mount.target, totalBytes, availableBytes, hostTotalBytes, hostAvailableBytes }
    } catch (error) {
      return { supported: false, reason: error instanceof Error ? error.message : String(error) }
    }
  }

  function quotaRow(projectId) {
    const current = requireSupported()
    const output = runXfsQuota(current.mountpoint, `quota -p -b -n -N ${projectId}`)
    return parseQuotaRow(output, projectId)
  }

  function requireSupported() {
    const result = probe()
    if (!result.supported) fail(result.reason)
    return result
  }

  function labelsMatch(volumeOrContainer, identity) {
    const labels = volumeOrContainer.Config?.Labels ?? volumeOrContainer.Labels ?? {}
    return labels["io.chariox.slice.id"] === identity.sliceId &&
      labels["io.chariox.slice.owner-kernel-id"] === identity.ownerKernelId &&
      labels["io.chariox.slice.owner-machine-id"] === identity.ownerMachineId
  }

  function inspectContainer(identity) {
    let value
    try { value = dockerJson(["container", "inspect", identity.containerName])[0] } catch (error) {
      if (String(error).includes("No such object") || String(error).includes("No such container")) return undefined
      throw error
    }
    if (!value || !labelsMatch(value, identity)) fail("Docker container labels do not match the disk quota reservation")
    const state = value.State?.Paused ? "paused" : value.State?.Running ? "running" : value.State?.Status ?? "unknown"
    return { state, labels: value.Config?.Labels ?? {} }
  }

  function inspectHome(identity) {
    const volume = dockerJson(["volume", "inspect", identity.homeVolumeName])[0]
    if (!volume || volume.Name !== identity.homeVolumeName || !labelsMatch(volume, identity)) fail("Docker home-volume labels do not match the disk quota reservation")
    if (volume.Driver !== "local" || !volume.Mountpoint) fail("persistent home is not a Docker local volume")
    const probeResult = requireSupported()
    const expected = `${canonicalDataRoot}/volumes/${identity.homeVolumeName}/_data`
    if (resolve(volume.Mountpoint) !== expected) fail("Docker home volume mountpoint escaped the pinned local-volume root")
    return {
      driver: volume.Driver,
      persistent: true,
      labels: volume.Labels ?? {},
      path: assertTrustedPath(volume.Mountpoint, `${canonicalDataRoot}/volumes/${identity.homeVolumeName}`, dockerUid),
      mountpoint: probeResult.mountpoint,
    }
  }

  function inspectLayer(identity) {
    const container = dockerJson(["container", "inspect", identity.containerName])[0]
    if (!container || !labelsMatch(container, identity)) fail("Docker container labels do not match the disk quota reservation")
    const graph = container.GraphDriver
    if (graph?.Name !== "overlay2" || typeof graph.Data?.UpperDir !== "string" || typeof graph.Data?.WorkDir !== "string") {
      fail("Docker classic overlay2 did not expose a stopped-container writable-layer mapping")
    }
    const upperDir = resolve(graph.Data.UpperDir)
    const workDir = resolve(graph.Data.WorkDir)
    if (upperDir !== graph.Data.UpperDir || workDir !== graph.Data.WorkDir) fail("Docker overlay2 upper/work paths are not canonical")
    const layerRoot = `${canonicalDataRoot}/overlay2`
    if (!upperDir.startsWith(`${layerRoot}/`) || !workDir.startsWith(`${layerRoot}/`)) fail("Docker overlay2 upper/work paths escaped the pinned data root")
    const layerPathPattern = new RegExp(`^${layerRoot}/[a-f0-9]{64}/(?:diff|work)$`)
    if (!layerPathPattern.test(upperDir) || !layerPathPattern.test(workDir)) fail("Docker overlay2 upper/work paths do not match the pinned layer layout")
    if (dirname(upperDir) !== dirname(workDir) || !upperDir.endsWith("/diff") || !workDir.endsWith("/work")) {
      fail("Docker overlay2 upper/work directory mapping is inconsistent")
    }
    return {
      driver: graph.Name,
      labels: container.Config?.Labels ?? {},
      paths: [
        assertTrustedPath(upperDir, layerRoot, dockerUid),
        assertTrustedPath(workDir, layerRoot, dockerUid),
      ],
      state: container.State?.Paused ? "paused" : container.State?.Running ? "running" : container.State?.Status ?? "unknown",
    }
  }

  function inspectQuotaTarget(paths, projectId) {
    const current = requireSupported()
    for (const path of paths) {
      const projectCheck = runXfsQuota(current.mountpoint, `project -c -p ${path} ${projectId}`)
      if (projectCheck.trim() !== "") fail("XFS project quota tree contains files without the reserved project ID")
    }
    return quotaRow(projectId)
  }

  function applyHardQuota({ storageClass, path, paths, projectId, limitBytes, state }) {
    const current = requireSupported()
    const trustedPaths = (paths ?? [path]).map((targetPath) => assertTrustedPath(
      targetPath,
      storageClass === "persistentHome"
        ? `${canonicalDataRoot}/volumes`
        : `${canonicalDataRoot}/overlay2`,
      dockerUid,
    ))
    const before = quotaRow(projectId)
    const treeMatches = trustedPaths.every((targetPath) =>
      runXfsQuota(current.mountpoint, `project -c -p ${targetPath} ${projectId}`).trim() === "",
    )
    const alreadyBound = treeMatches && before.found && before.hardLimitBytes === limitBytes
    if (!alreadyBound && !["absent", "created", "exited", "dead"].includes(state)) {
      fail(`cannot add or change ${storageClass} quota unless its container is stopped`)
    }
    if (!alreadyBound) {
      for (const targetPath of trustedPaths) {
        runXfsQuota(current.mountpoint, `project -s -p ${targetPath} ${projectId}`)
      }
      const assigned = quotaRow(projectId)
      if (!assigned.found || !Number.isSafeInteger(assigned.usedBytes) || assigned.usedBytes < 0) {
        fail(`${storageClass} usage could not be read back after project assignment`)
      }
      const assignedUsage = assigned.usedBytes
      if (assignedUsage > limitBytes) fail(`existing ${storageClass} data exceeds its configured hard quota`)
      runXfsQuota(current.mountpoint, `limit -p bhard=${limitBytes} ${projectId}`)
    }
    return verifyHardQuota({ storageClass, paths: trustedPaths, projectId, limitBytes })
  }

  function verifyHardQuota({ path, paths, projectId, limitBytes }) {
    const quota = inspectQuotaTarget((paths ?? [path]).map((targetPath) => resolve(targetPath)), projectId)
    if (!quota.found || quota.hardLimitBytes !== limitBytes) fail("XFS hard quota readback does not equal its durable reservation")
    return { treeVerified: true, effectiveLimitBytes: quota.hardLimitBytes, usedBytes: quota.usedBytes }
  }

  function projectIdsInUse() {
    const current = requireSupported()
    const output = runXfsQuota(current.mountpoint, "report -p -n -N")
    const ids = []
    for (const line of output.split(/\r?\n/)) {
      const match = /^\s*(\d+)\s+/.exec(line)
      if (match) ids.push(Number(match[1]))
    }
    return ids
  }

  function projectUsageBytes(projectId) {
    const row = quotaRow(projectId)
    return row.found ? row.usedBytes : undefined
  }

  function clearProjectQuota(projectId) {
    const current = requireSupported()
    const used = quotaRow(projectId)
    if (used.found && used.usedBytes !== 0) fail("Docker storage still has project-quota usage; reservation remains allocated")
    if (!used.found && projectIdsInUse().includes(projectId)) {
      fail("XFS project-quota usage could not be read back; reservation remains allocated")
    }
    if (!used.found) return
    runXfsQuota(current.mountpoint, `limit -p bhard=0 ${projectId}`)
    const after = quotaRow(projectId)
    if (after.usedBytes !== 0 || after.hardLimitBytes !== 0) fail("XFS project quota release readback failed")
  }

  function confirmContainerAndVolumeRemoved(identity) {
    const exists = (kind, name) => {
      try {
        const objects = dockerJson([kind, "inspect", name])
        if (!Array.isArray(objects) || objects.length !== 1 || typeof objects[0]?.Name !== "string") {
          fail(`Docker ${kind} removal state could not be verified`)
        }
        if (objects[0].Name !== name && objects[0].Name !== `/${name}`) {
          fail(`Docker ${kind} inspection returned a different identity`)
        }
        return true
      } catch (error) {
        const message = String(error).toLowerCase()
        if (message.includes(`no such ${kind === "container" ? "container" : "volume"}`) || message.includes("no such object")) {
          return false
        }
        throw error
      }
    }
    return !exists("container", identity.containerName) && !exists("volume", identity.homeVolumeName)
  }

  return {
    probe,
    inspectHome,
    inspectLayer,
    inspectContainer,
    applyHardQuota,
    verifyHardQuota,
    projectIdsInUse,
    projectUsageBytes,
    clearProjectQuota,
    confirmContainerAndVolumeRemoved,
  }
}

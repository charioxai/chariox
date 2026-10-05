export function browserStateCleanupFailure(result) {
  const leaks = []
  if (!result.dockerAvailable) leaks.push("Docker verification unavailable")
  if (!result.containerGone) leaks.push("container")
  if (result.fixtureSidecarGone === false) leaks.push("fixture sidecar")
  if (!result.volumeGone) leaks.push("volume")
  if (!result.savedImageGone) leaks.push("saved image")
  if (result.backupImagesGone === false) leaks.push("backup images")
  if (result.rollbackImagesUnresolved?.length) leaks.push(`unresolved rollback images ${result.rollbackImagesUnresolved.join(", ")}`)
  if (result.rollbackImageInventoryFailed) leaks.push("rollback image inventory unavailable")
  if (result.stateImageInventoryFailed) leaks.push("state image inventory unavailable")
  if (result.fixtureWorkspaceRemoved === false) leaks.push("fixture workspace")
  if (!result.tempRootRemoved) leaks.push("runtime root")
  if (!result.listenersReleased) {
    leaks.push(`ports ${(result.occupiedPorts ?? []).join(", ") || "unknown"}`)
  }
  return leaks.length === 0
    ? null
    : new Error(`browser state drill cleanup leaked: ${leaks.join("; ")}`)
}

export async function cleanupBrowserStateImages({ imageRefs, imagesBefore, slice, inspectImage, removeImage }) {
  const candidates = [...new Set(imageRefs)].filter(imageRef => !imagesBefore.has(imageRef))
  const result = { removedImageRefs: [], preservedImageRefs: [], unresolvedImageRefs: [] }
  const ownership = {
    "io.chariox.slice.id": slice?.id,
    "io.chariox.slice.owner-kernel-id": slice?.owner_kernel_id,
    "io.chariox.slice.owner-machine-id": slice?.owner_machine_id,
  }
  if (!Object.values(ownership).every(value => typeof value === "string" && value.trim())) {
    result.unresolvedImageRefs.push(...candidates)
    return result
  }
  for (const imageRef of candidates) {
    try {
      const image = await inspectImage(imageRef)
      if (!image) continue
      if (!Object.entries(ownership).every(([key, value]) => image.Config?.Labels?.[key] === value)) {
        result.preservedImageRefs.push(imageRef)
        continue
      }
      if (!/^sha256:[a-f0-9]{64}$/.test(image.Id ?? "")) {
        result.unresolvedImageRefs.push(imageRef)
        continue
      }
      // The tag may move after inspection. Remove only the owned immutable ID.
      await removeImage(image.Id)
      if (await inspectImage(image.Id)) result.unresolvedImageRefs.push(imageRef)
      else result.removedImageRefs.push(imageRef)
    } catch {
      result.unresolvedImageRefs.push(imageRef)
    }
  }
  return result
}

export async function assertBrowserStateDockerNamesAvailable({ containerName, homeVolume, inspect }) {
  for (const [kind, name] of [["container", containerName], ["volume", homeVolume]]) {
    if (await inspect(kind, name)) throw new Error(`refusing pre-existing drill ${kind}: ${name}`)
  }
}

export async function captureBrowserStateDockerOwnership({ runId, containerName, homeVolume, slice, inspect }) {
  if (!runId || !containerName.includes(runId) || !homeVolume.includes(runId)) throw new Error("Docker drill names are not bound to this unique run")
  const labels = {
    "io.chariox.slice.id": slice?.id,
    "io.chariox.slice.owner-kernel-id": slice?.owner_kernel_id,
    "io.chariox.slice.owner-machine-id": slice?.owner_machine_id,
  }
  if (!Object.values(labels).every(value => typeof value === "string" && value.trim())) throw new Error("missing Docker drill ownership labels")
  const container = await inspect("container", containerName)
  const volume = await inspect("volume", homeVolume)
  if (!/^[a-f0-9]{64}$/.test(container?.Id ?? "") || !labelsMatch(container.Config?.Labels, labels)
    || volume?.Name !== homeVolume || !volume.CreatedAt || !volume.Mountpoint || !labelsMatch(volume.Labels, labels)) throw new Error("Docker drill resources do not match the created slice")
  return { runId, containerId: container.Id, labels, volume: volumeIdentity(volume) }
}

function labelsMatch(actual, expected) {
  return Object.entries(expected).every(([key, value]) => actual?.[key] === value)
}

function volumeIdentity(volume) {
  return { Name: volume.Name, CreatedAt: volume.CreatedAt, Driver: volume.Driver, Mountpoint: volume.Mountpoint, Labels: volume.Labels }
}

export async function cleanupBrowserStateDockerResources({ containerName, homeVolume, ownership, inspect, remove }) {
  if (!ownership || !containerName.includes(ownership.runId) || !homeVolume.includes(ownership.runId)) return
  const container = await inspect("container", containerName)
  if (container?.Id === ownership.containerId && labelsMatch(container.Config?.Labels, ownership.labels)) {
    const current = await inspect("container", ownership.containerId)
    if (current?.Id === ownership.containerId && labelsMatch(current.Config?.Labels, ownership.labels)) await remove(["rm", "-f", ownership.containerId])
  }
  const sameVolume = (volume) => volume && labelsMatch(volume.Labels, ownership.labels)
    && JSON.stringify(volumeIdentity(volume)) === JSON.stringify(volumeIdentity(ownership.volume))
  if (sameVolume(await inspect("volume", homeVolume)) && sameVolume(await inspect("volume", homeVolume))) await remove(["volume", "rm", "-f", homeVolume])
}

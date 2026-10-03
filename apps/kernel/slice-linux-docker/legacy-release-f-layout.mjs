// Release F capture compatibility is an explicit, retained broker decision.
// A missing protected receipt is never sufficient to enable it.
import { PRIVATE_ROOT } from "./protected-layout.mjs"

export const LEGACY_CAPTURE_NOTICE = "Legacy release F slice: save/backup includes the original mixed home and image, which may contain credentials. Migrate to a protected slice to enable credential-separated capture."
export function verifyLegacyReleaseFLayout(container, info, image) {
  const version = image?.Config?.Labels?.["io.chariox.relay-peer-protocol-version"]
  const homes = info?.Mounts?.filter(mount => mount.Destination === "/home/slice")
  if (!/^chariox-slice-[A-Za-z0-9_.:-]+$/.test(container)
      || typeof info?.Id !== "string" || !info.Id || image?.Id !== info.Image
      || !/^(?:5[8-9]|6[0-8])$/.test(version ?? "")
      || info.Mounts?.some(mount => mount.Destination === PRIVATE_ROOT || mount.Destination?.startsWith(`${PRIVATE_ROOT}/`))
      || info.Config?.Env?.some(value => value.startsWith("CHARIOX_SLICE_PRIVATE_ROOT="))
      || homes?.length !== 1 || homes[0].Type !== "volume" || homes[0].Name !== `${container}-home`) {
    throw new Error("Slice is not a verified legacy release F layout; protected capture remains required")
  }
  return {version: 1, sliceId: container, layoutKind: "legacy-release-f", containerId: info.Id,
    imageId: info.Image, homeVolume: homes[0].Name, homeSource: homes[0].Source}
}

import { isAbsolute, normalize } from "node:path"

export const PRIVATE_ROOT = "/var/lib/chariox/slice-private"
export const PRIVATE_ENVIRONMENT = Object.freeze({
  CHARIOX_HOME: `${PRIVATE_ROOT}/kernel`,
  CHARIOX_MANAGED_PROVIDER_HOME: `${PRIVATE_ROOT}/provider-home`,
  CHARIOX_SLICE_PRIVATE_ROOT: PRIVATE_ROOT,
  GH_CONFIG_DIR: `${PRIVATE_ROOT}/provider-home/.config/gh`,
})

const PUBLIC_ENVIRONMENT = new Set([
  "PATH", "HOME", "USER", "LANG", "LC_ALL", "DISPLAY", "TERM", "SHELL",
  "DEBIAN_FRONTEND", "NODE_VERSION", "YARN_VERSION", "CHARIOX_SLICE_ROOT", "CHARIOX_SLICE_SELKIES_BIN",
  "CHARIOX_SLICE_VIEWER_BACKEND", "CHARIOX_SLICE_DISPLAY_MODE", "CHARIOX_SLICE_DISPLAY_SERVER",
  "CHARIOX_SLICE_NOVNC_PORT", "CHARIOX_SLICE_SCREEN_GEOMETRY",
  "CHARIOX_SLICE_MIN_FREE_MB", "CHARIOX_MANAGED_WORKSPACE_ROOT_COUNT",
  ...Object.keys(PRIVATE_ENVIRONMENT),
])

function refuse() {
  throw new Error("Slice save/backup is unavailable for this storage layout; existing saved state is preserved")
}

export function verifyHomeVolumeName(volume, owner) {
  if (typeof volume !== "string" || !/^chariox-slice-[A-Za-z0-9_.:-]+-home(?:-g[a-f0-9]{32})?$/.test(volume)) refuse()
  if (owner !== undefined && !(volume === `${owner}-home`
      || (volume.startsWith(`${owner}-home-g`) && /^[a-f0-9]{32}$/.test(volume.slice(`${owner}-home-g`.length))))) refuse()
}

// The receipt is supplied by the trusted host provisioner, never container labels.
// Filesystem ownership and the signed build-context proof are verified by its caller.
export function verifyProtectedCaptureLayout(inspect, receipt, trustedBaseDigests) {
  if (!receipt || receipt.version !== 1 || !trustedBaseDigests.has(receipt.baseImageId)) refuse()
  if (inspect?.Id !== receipt.containerId || inspect?.Image !== receipt.imageId) refuse()
  if (!/^sha256:[a-f0-9]{64}$/.test(receipt.imageId)) refuse()
  if (!isAbsolute(receipt.privateHostRoot) || normalize(receipt.privateHostRoot) !== receipt.privateHostRoot) refuse()
  const mounts = inspect.Mounts
  if (!Array.isArray(mounts)) refuse()
  const privateMount = mounts.find(m => m.Destination === PRIVATE_ROOT)
  const home = mounts.find(m => m.Destination === "/home/slice")
  const nss = mounts.find(m => m.Destination === "/home/slice/.local/share/pki/nssdb")
  if (privateMount?.Type !== "bind" || privateMount.Source !== receipt.privateHostRoot || privateMount.RW !== true) refuse()
  if (home?.Type !== "volume" || home.Name !== receipt.homeVolume || home.RW !== true) refuse()
  if (receipt.homeSource !== undefined && home.Source !== receipt.homeSource) refuse()
  if (nss?.Type !== "bind" || nss.Source !== `${receipt.privateHostRoot}/nssdb` || nss.RW !== true) refuse()
  for (const mount of mounts) {
    const destination = mount.Destination
    if (typeof destination !== "string" || !isAbsolute(destination) || normalize(destination) !== destination) refuse()
    // A parent mount can replace trusted runtime code just as a leaf mount can.
    const runtime = "/opt/chariox-slice"
    if (destination === "/" || destination === runtime || runtime.startsWith(`${destination}/`)
        || destination.startsWith(`${runtime}/`)) refuse()
    if (mount.Destination.startsWith(`${PRIVATE_ROOT}/`) ||
        (mount.Destination.startsWith("/home/slice/") && mount !== nss)) refuse()
  }
  const environment = inspect.Config?.Env
  if (!Array.isArray(environment)) refuse()
  const values = new Map()
  for (const assignment of environment) {
    const separator = assignment.indexOf("=")
    if (separator < 1) refuse()
    const name = assignment.slice(0, separator)
    if (values.has(name) || (!PUBLIC_ENVIRONMENT.has(name) && !/^CHARIOX_MANAGED_WORKSPACE_ROOT_[0-9]+$/.test(name))) refuse()
    const value = assignment.slice(separator + 1)
    if (["NODE_VERSION", "YARN_VERSION"].includes(name) && !/^[0-9]{1,3}\.[0-9]{1,3}\.[0-9]{1,3}$/.test(value)) refuse()
    if (name === "CHARIOX_SLICE_DISPLAY_SERVER" && !["xorg", "xvfb"].includes(value.toLowerCase())) refuse()
    values.set(name, value)
  }
  for (const [name, value] of Object.entries(PRIVATE_ENVIRONMENT)) {
    if (values.get(name) !== value) refuse()
  }
  if (values.get("HOME") !== "/home/slice") refuse()
  return { privateHostRoot: receipt.privateHostRoot, homeVolume: receipt.homeVolume }
}

// Known managed roots only. This does not detect arbitrarily named user secrets.
export function requireSupportedHomeEntries(paths) {
  const forbidden = [".codex", ".claude", ".claude.json", ".ssh", ".gnupg", ".config/gh", ".local/share/opencode", ".config/chariox", ".local/state/chariox", ".local/share/pki/nssdb/key4.db", ".chariox/provider-home", ".chariox/keys"]
  for (const path of paths) {
    if (path.startsWith("/") || path.split("/").includes("..")) refuse()
    const canonical = normalize(path)
    if (canonical.startsWith(".chariox/") && canonical !== ".chariox/browser" && !canonical.startsWith(".chariox/browser/")) refuse()
    if (forbidden.some(root => canonical === root || canonical.startsWith(`${root}/`))) refuse()
  }
}

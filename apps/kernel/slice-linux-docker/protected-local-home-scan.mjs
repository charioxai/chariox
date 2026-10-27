import { randomBytes } from "node:crypto"
import { requireSupportedHomeEntries, verifyHomeVolumeName } from "./protected-layout.mjs"
import { verifyHomeEntryMetadata } from "./protected-home-capture.mjs"

function refuse() { throw new Error("Local slice home inventory was refused; saved state is preserved") }
export function verifyLocalHomeInventory(bytes, {quiesced = true, maxEntries = 100_000} = {}) {
  const fields = Buffer.from(bytes).toString("utf8").split("\0")
  if (fields.pop() !== "" || fields.length % 4 || fields.length / 4 > maxEntries) refuse()
  for (let index = 0; index < fields.length; index += 4) {
    const [path, kind, target, mode] = fields.slice(index, index + 4)
    requireSupportedHomeEntries([path])
    if (!/^[0-7]{3,4}$/.test(mode) || (Number.parseInt(mode, 8) & 0o6000)) refuse()
    if (quiesced) verifyHomeEntryMetadata(Buffer.from(`${path}\0${kind}\0${target}\0`))
  }
}

// Inspect the exact volume through the selected, attested public worker image.
// The daemon data-root is never mounted into the broker or addressed directly.
export function requireSafeLocalHomeVolume({volume, docker, enrollment, quiesced = true, maxEntries}) {
  verifyHomeVolumeName(volume)
  const inspected = docker(["volume", "inspect", volume])
  if (inspected.status !== 0) refuse()
  const records = JSON.parse(inspected.stdout)
  if (!Array.isArray(records) || records.length !== 1 || records[0].Name !== volume
      || records[0].Driver !== "local" || records[0].Scope !== "local"
      || (records[0].Options && Object.keys(records[0].Options).length)) refuse()
  const helper = `chariox-local-home-scan-${enrollment.ownerUid}-${randomBytes(12).toString("hex")}`
  try {
    const inventory = docker(["run", "--rm", "--name", helper, "--read-only", "--network", "none",
      "--user", "0:0", "--cap-drop", "ALL", "--cap-add", "DAC_OVERRIDE", "--security-opt", "no-new-privileges",
      "--memory", "64m", "--pids-limit", "16", "--mount", `type=volume,src=${volume},dst=/home-src,readonly`,
      "--entrypoint", "/usr/bin/find", enrollment.workerImageId,
      "-P", "/home-src", "-mindepth", "1", "-printf", "%P\\0%y\\0%l\\0%m\\0"])
    if (inventory.status !== 0) refuse()
    verifyLocalHomeInventory(inventory.stdout, {quiesced, maxEntries})
  } finally {
    docker(["rm", "-f", helper])
  }
}

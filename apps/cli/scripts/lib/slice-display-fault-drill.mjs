import path from "node:path"

export const DISPLAY_FAULT_CASE_IDS = Object.freeze([
  "fault.streamer-crash",
  "fault.browser-crash",
  "cleanup.resources",
])

const PROBE_SCHEMA = "chariox.slice_display_fault_probe.v1"

export function buildDisplayFaultDockerArgs({ containerName, image, sourceRoot }) {
  requireText(containerName, "containerName")
  requireText(image, "image")
  if (!path.isAbsolute(sourceRoot)) throw new Error("sourceRoot must be absolute")
  if (sourceRoot.includes(",")) throw new Error("sourceRoot cannot contain a comma")
  return [
    "run",
    "--rm",
    "--name",
    containerName,
    "--network",
    "none",
    "--memory",
    "1g",
    "--memory-swap",
    "1g",
    "--cpus",
    "1",
    "--pids-limit",
    "256",
    "--read-only",
    "--security-opt",
    "no-new-privileges",
    "--tmpfs",
    "/tmp:rw,nosuid,nodev,size=384m,mode=1777",
    "--tmpfs",
    "/home/slice:rw,nosuid,nodev,size=128m,uid=1001,gid=1001,mode=0700",
    "--mount",
    `type=bind,src=${sourceRoot},dst=/drill,readonly`,
    "--entrypoint",
    "/opt/chariox-selkies/bin/python",
    image,
    "/drill/validate-slice-viewer.py",
    "--json",
  ]
}

export function validateDisplayFaultProbe(probe) {
  requireExactObject(probe, "display fault probe", ["schema", "streamer", "browser", "cleanup"])
  if (probe.schema !== PROBE_SCHEMA) throw new Error(`unsupported display fault probe schema ${JSON.stringify(probe.schema)}`)
  requireExactObject(probe.streamer, "display fault probe.streamer", [
    "crashDetected",
    "browserRemainedAvailable",
    "recoveredOnce",
  ])
  requireExactObject(probe.browser, "display fault probe.browser", [
    "crashDetected",
    "streamerRemainedAvailable",
    "recoveredOnce",
    "profileStatePreserved",
  ])
  requireExactObject(probe.cleanup, "display fault probe.cleanup", [
    "displaySocketRemoved",
    "portsReleased",
  ])
  for (const [section, fields] of Object.entries({
    streamer: ["crashDetected", "browserRemainedAvailable", "recoveredOnce"],
    browser: ["crashDetected", "streamerRemainedAvailable", "recoveredOnce", "profileStatePreserved"],
    cleanup: ["displaySocketRemoved", "portsReleased"],
  })) {
    for (const field of fields) {
      if (probe[section][field] !== true) throw new Error(`display fault probe.${section}.${field} must be true`)
    }
  }
  return probe
}

function requireExactObject(value, label, keys) {
  if (!value || typeof value !== "object" || Array.isArray(value)) throw new Error(`${label} must be an object`)
  const actual = Object.keys(value).sort()
  const expected = [...keys].sort()
  if (JSON.stringify(actual) !== JSON.stringify(expected)) throw new Error(`${label} has unexpected keys`)
}

function requireText(value, label) {
  if (typeof value !== "string" || value.trim() === "") throw new Error(`${label} must be non-empty text`)
}

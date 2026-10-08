import { readFileSync, readdirSync } from "node:fs"
import { homedir } from "node:os"
import { join } from "node:path"

const presenceSchemaVersion = 1
export const localKernelPresenceFreshnessMs = 30_000
const maximumLocalKernelPresences = 128

export type LocalKernelPresence = {
  readonly kernelId: string
  readonly kernelAlias?: string | null
  readonly machineId: string
  readonly machineAlias?: string | null
  readonly host: string
  readonly port: number
  readonly heartbeatAtMs: number
  readonly protocolVersion?: number
}

export function loadLocalKernelPresences(
  directory = defaultActiveKernelRegistryDir(),
  nowMs = Date.now(),
): LocalKernelPresence[] {
  try {
    return readdirSync(directory, { withFileTypes: true })
      .filter((entry) => entry.isFile() && entry.name.endsWith(".json"))
      .flatMap((entry) => readPresence(join(directory, entry.name), nowMs))
      .sort((left, right) => right.heartbeatAtMs - left.heartbeatAtMs)
      .slice(0, maximumLocalKernelPresences)
  } catch {
    return []
  }
}

export function localKernelEndpoint(presence: LocalKernelPresence): string {
  const host = presence.host.includes(":") ? `[${presence.host}]` : presence.host
  return `ws://${host}:${presence.port}/kernel`
}

function readPresence(path: string, nowMs: number): LocalKernelPresence[] {
  try {
    const record = JSON.parse(readFileSync(path, "utf8")) as {
      readonly schema_version?: number
      readonly kernel_id?: string
      readonly kernel_alias?: string | null
      readonly machine_id?: string
      readonly machine_alias?: string | null
      readonly host?: string
      readonly port?: number
      readonly heartbeat_at_ms?: number
      readonly local_daemon_protocol_version?: number
    }
    if (
      record.schema_version !== presenceSchemaVersion
      || !record.kernel_id?.trim()
      || !record.machine_id?.trim()
      || !record.host?.trim()
      || !Number.isInteger(record.port)
      || (record.port ?? 0) < 1
      || (record.port ?? 0) > 65_535
      || !record.heartbeat_at_ms
      || Math.abs(nowMs - record.heartbeat_at_ms) > localKernelPresenceFreshnessMs
    ) {
      return []
    }
    return [{
      kernelId: record.kernel_id.trim(),
      kernelAlias: record.kernel_alias?.trim() || null,
      machineId: record.machine_id.trim(),
      machineAlias: record.machine_alias?.trim() || null,
      host: record.host.trim(),
      port: record.port as number,
      heartbeatAtMs: record.heartbeat_at_ms,
      ...(Number.isInteger(record.local_daemon_protocol_version) && (record.local_daemon_protocol_version ?? 0) > 0
        ? { protocolVersion: record.local_daemon_protocol_version } : {}),
    }]
  } catch {
    return []
  }
}

function defaultActiveKernelRegistryDir(): string {
  const explicit = process.env.CHARIOX_ACTIVE_KERNEL_REGISTRY_DIR?.trim()
  if (explicit) {
    return explicit
  }
  const charioxHome = process.env.CHARIOX_HOME?.trim()
  if (charioxHome) return join(charioxHome, "kernels", "active")
  const xdgConfigHome = process.env.XDG_CONFIG_HOME?.trim()
  return xdgConfigHome
    ? join(xdgConfigHome, "chariox", "kernels", "active")
    : join(homedir(), ".chariox", "kernels", "active")
}

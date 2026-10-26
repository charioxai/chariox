import { spawnSync } from 'node:child_process'
import { closeSync, openSync } from 'node:fs'
import { readFile, statfs } from 'node:fs/promises'

export function run(command, args, { cwd, env = process.env, log, input, timeout = 60 * 60_000 } = {}) {
  const descriptor = log ? openSync(log, 'a', 0o600) : undefined
  try {
    const result = spawnSync(command, args, { cwd, env, input, timeout, maxBuffer: 8 * 1024 * 1024,
      stdio: [input === undefined ? 'ignore' : 'pipe', descriptor ?? 'pipe', descriptor ?? 'pipe'] })
    if (result.error || result.status !== 0) throw Error(`command failed: ${command} (exit ${result.status ?? 'unsettled'}); inspect private build log`)
    return result.stdout?.toString().trim() ?? ''
  } finally { if (descriptor !== undefined) closeSync(descriptor) }
}

export async function resourceGuard(directory, { statfsImpl = statfs, readFileImpl = readFile } = {}) {
  const disk = await statfsImpl(directory)
  if (disk.bavail / disk.blocks < 0.15) throw Error('MP-07 / MP-11: pause; disk free below 15%')
  if (process.platform === 'linux') {
    const memory = await readFileImpl('/proc/meminfo', 'utf8')
    const available = Number(memory.match(/^MemAvailable:\s+(\d+)/m)?.[1] ?? 0) * 1024
    if (available < 12 * 1024 ** 3) throw Error('MP-07 / MP-11: pause; MemAvailable below 12 GiB')
  }
}

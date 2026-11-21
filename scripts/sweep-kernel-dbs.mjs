import { readdirSync, realpathSync, statSync } from 'node:fs'
import { basename, dirname, join, resolve } from 'node:path'
import { parseArgs } from 'node:util'

function directory(path) {
  const resolved = realpathSync(path)
  if (!statSync(resolved).isDirectory()) throw new Error(`${path} is not a directory`)
  return resolved
}

function kernelDb(path) {
  if (basename(path) !== 'kernel.db') throw new Error(`${path} must name a kernel.db file`)
  return join(directory(dirname(resolve(path))), 'kernel.db')
}

try {
  const { values, positionals } = parseArgs({
    options: { 'kernel-db': { type: 'string', multiple: true, default: [] } },
    allowPositionals: true,
  })
  if (positionals.length === 0) {
    throw new Error('Usage: node scripts/sweep-kernel-dbs.mjs [--kernel-db <configured kernel.db>]... <scan-root>...')
  }

  const configured = new Set(values['kernel-db'].map(kernelDb))
  const pending = positionals.map(directory)
  const visited = new Set()
  const misplaced = []
  while (pending.length) {
    const dir = pending.pop()
    if (visited.has(dir)) continue
    visited.add(dir)
    for (const entry of readdirSync(dir, { withFileTypes: true })) {
      const path = join(dir, entry.name)
      if (entry.isDirectory()) pending.push(path)
      if (entry.name !== 'kernel.db') continue
      if (entry.isSymbolicLink() || (entry.isFile() && !configured.has(path))) misplaced.push(path)
    }
  }
  for (const path of misplaced.sort()) console.log(path)
  process.exitCode = misplaced.length ? 1 : 0
} catch (error) {
  console.error(error.message)
  process.exitCode = 2
}

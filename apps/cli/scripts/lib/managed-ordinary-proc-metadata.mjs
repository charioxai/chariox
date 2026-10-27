// MP-10 read-only Linux metadata inspection for nondumpable kernels. The
// collector and its permission probes keep the provider's original user.
import { execFile } from 'node:child_process'
import { promisify } from 'node:util'
import { createHash } from 'node:crypto'
import { readFile, readdir, readlink } from 'node:fs/promises'
import { fileURLToPath } from 'node:url'
import { resolve } from 'node:path'

const helperPath = fileURLToPath(import.meta.url)
const execute = promisify(execFile)

function numeric(value) {
  if (!/^[1-9][0-9]{0,19}$/.test(String(value))) throw new Error('MP-10 invalid process metadata identifier')
  return String(value)
}

async function inspect(action, value) {
  const id = numeric(value)
  if (process.platform !== 'linux' || process.getuid?.() !== 0) throw new Error('MP-10 privileged Linux metadata inspector required')
  if (action === 'process') {
    const path = `/proc/${id}`
    const stat = await readFile(`${path}/stat`, 'utf8')
    const executableLink = await readlink(`${path}/exe`)
    const executableDigest = `sha256:${createHash('sha256').update(await readFile(`${path}/exe`)).digest('hex')}`
    const after = await readFile(`${path}/stat`, 'utf8')
    const ticks = text => text.slice(text.lastIndexOf(')') + 2).trim().split(/\s+/)[19]
    if (ticks(stat) !== ticks(after) || executableLink !== await readlink(`${path}/exe`)) throw new Error('MP-10 process changed during inspection')
    return { pid: Number(id), stat: after, bootId: await readFile('/proc/sys/kernel/random/boot_id', 'utf8'), executableLink, executableDigest }
  }
  if (action === 'socket-owner') {
    const target = `socket:[${id}]`, owners = new Set()
    for (const pid of await readdir('/proc')) {
      if (!/^[1-9][0-9]*$/.test(pid)) continue
      let fds
      try { fds = await readdir(`/proc/${pid}/fd`) } catch (error) {
        if (['ENOENT', 'ESRCH'].includes(error.code)) continue
        throw error
      }
      for (const fd of fds) {
        try { if (await readlink(`/proc/${pid}/fd/${fd}`) === target) owners.add(Number(pid)) } catch (error) {
          if (!['ENOENT', 'ESRCH'].includes(error.code)) throw error
        }
      }
    }
    return { owners: [...owners] }
  }
  throw new Error('MP-10 unsupported metadata operation')
}

export async function inspectPrivilegedProcMetadata(action, value) {
  numeric(value)
  if (!['process', 'socket-owner'].includes(action)) throw new Error('MP-10 unsupported metadata operation')
  if (process.getuid?.() === 0) return inspect(action, value)
  // No provider environment or credentials enter the privileged helper.
  const { stdout } = await execute('sudo', ['-n', process.execPath, helperPath, action, String(value)], {
    env: { PATH: '/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin', LANG: 'C' },
    timeout: 10000, maxBuffer: 128 * 1024,
  })
  return JSON.parse(stdout)
}

if (process.argv[1] && resolve(process.argv[1]) === helperPath) {
  try { console.log(JSON.stringify(await inspect(process.argv[2], process.argv[3]))) }
  catch { console.error('MP-10 metadata inspection failed'); process.exitCode = 1 }
}

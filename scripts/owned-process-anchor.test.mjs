// MP-11 #868: Darwin ps must not keep the retained launch anchor alive itself.
import assert from 'node:assert/strict'
import { test } from 'node:test'
import { once } from 'node:events'
import { mkdtemp, readFile, writeFile, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { spawnOwned, signalOwnedProcessGroup } from '../apps/kernel/slice-linux-docker/owned-process-signals.mjs'

const observer = new URL('../apps/cli/scripts/lib/managed-browser-computer-parity-host-observer.mjs', import.meta.url).href
const supported = { skip: !['linux', 'darwin'].includes(process.platform), timeout: 10_000 }

for (const descendant of [false, true]) {
  test(`MP-11 Darwin retained observer closes normally${descendant ? ' after a descendant named ps settles' : ''}`, supported, async () => {
    const root = await mkdtemp(join(tmpdir(), 'chariox-mp11-darwin-anchor-'))
    let child, closed
    try {
      // Linux ps accepts these same public metadata columns. Force only the
      // platform branch there; on Darwin this exercises the native path as is.
      const preload = join(root, 'darwin.mjs')
      await writeFile(preload, "Object.defineProperty(process, 'platform', { value: 'darwin' })\n")
      const marker = join(root, 'settled')
      const background = `process.title = 'ps'; setTimeout(() => { require('node:fs').writeFileSync(${JSON.stringify(marker)}, 'settled'); }, 250)`
      const command = descendant
        ? `require('node:child_process').spawn(process.execPath, ['-e', ${JSON.stringify(background)}], { stdio: 'ignore' }); process.stdout.write('success'); process.exit(0)`
        : "process.stdout.write('success')"
      const worker = `
        import { runBoundedObserverCommand } from ${JSON.stringify(observer)};
        try {
          const output = await runBoundedObserverCommand(process.execPath, ['-e', ${JSON.stringify(command)}], { timeoutMs: 2000 });
          process.stdout.write(output);
        } catch (error) { process.stderr.write(error.message); process.exitCode = 1; }
      `
      child = spawnOwned(process.execPath, ['--input-type=module', '-e', worker], {
        detached: true, stdio: ['ignore', 'pipe', 'pipe'],
        env: { ...process.env, ...(process.platform === 'linux' ? { NODE_OPTIONS: `--import=${preload}` } : {}) },
      })
      closed = once(child, 'close')
      let stdout = '', stderr = ''
      child.stdout.on('data', chunk => { stdout += chunk })
      child.stderr.on('data', chunk => { stderr += chunk })
      const [code, signal] = await closed
      assert.equal(code, 0, stderr)
      assert.equal(signal, null)
      assert.equal(stdout, 'success')
      assert.equal(stderr, '')
      if (descendant) assert.equal(await readFile(marker, 'utf8'), 'settled', 'MP-11 do not broadly exempt children or processes named ps')
    } finally {
      if (child) { signalOwnedProcessGroup(child, 'SIGKILL'); await closed }
      await rm(root, { recursive: true, force: true })
    }
  })
}

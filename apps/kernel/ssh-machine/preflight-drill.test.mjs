// MP-07/MP-08/MP-11: real SSH seam, supplementary conflict regression.
import assert from 'node:assert/strict'
import { createServer } from 'node:net'
import { mkdir, writeFile, readFile } from 'node:fs/promises'
import { join } from 'node:path'
import test from 'node:test'
import { runSshMachine } from './transport.mjs'
import { sshTarget, availablePort } from './ssh-target-fixture.mjs'

for (const conflict of ['port', 'mcp-port', 'unit']) {
  test(`MP-07/MP-08/MP-11 ${conflict} refused before opening archive or public pins`, async t => {
    const { home } = await sshTarget(t)
    const port = await availablePort()
    const request = { action: 'install', installId: 'byom-one', port, releaseDigest: `sha256:${'a'.repeat(64)}` }
    if (conflict === 'unit') {
      await mkdir(join(home, '.config/systemd/user'), { recursive: true, mode: 0o700 })
      await writeFile(join(home, '.config/systemd/user/chariox-ssh-byom-one.service'), 'foreign unit', { mode: 0o600 })
    } else {
      const server = createServer()
      await new Promise(resolve => server.listen(port + (conflict === 'mcp-port' ? 1 : 0), '127.0.0.1', resolve))
      t.after(() => new Promise(resolve => server.close(resolve)))
    }
    await assert.rejects(runSshMachine('byom-local', request, { archive: '/nonexistent/archive', releasePublicKey: '/nonexistent/public-pin' }), error => {
      assert.equal(error.exitCode, conflict === 'unit' ? 77 : conflict === 'mcp-port' ? 78 : 76)
      return true
    })
    assert.equal((await readFile(join(home, 'fixture-service-calls'), 'utf8')).includes('daemon-reload'), false)
  })
}

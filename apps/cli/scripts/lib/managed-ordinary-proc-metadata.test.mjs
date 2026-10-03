import assert from 'node:assert/strict'
import { readFile, readlink } from 'node:fs/promises'
import { createHash } from 'node:crypto'
import { test } from 'node:test'
import { inspectPrivilegedProcMetadata } from './managed-ordinary-proc-metadata.mjs'

test('MP-10 privileged inspector returns only actual Linux executable metadata', { skip: process.platform !== 'linux' || process.getuid?.() !== 0 }, async () => {
  const result = await inspectPrivilegedProcMetadata('process', process.pid)
  assert.deepEqual(Object.keys(result).sort(), ['bootId', 'executableDigest', 'executableLink', 'pid', 'stat'].sort())
  assert.equal(result.pid, process.pid)
  assert.equal(result.executableLink, await readlink('/proc/self/exe'))
  assert.equal(result.executableDigest, `sha256:${createHash('sha256').update(await readFile('/proc/self/exe')).digest('hex')}`)
  assert.equal(result.bootId, await readFile('/proc/sys/kernel/random/boot_id', 'utf8'))
})

test('MP-10 privileged inspector rejects arbitrary paths, operations and malformed process IDs', async () => {
  for (const id of ['../environ', '/etc/passwd', '1;id', '0', '-1', '1.5', '']) {
    await assert.rejects(inspectPrivilegedProcMetadata('process', id), /invalid process metadata identifier/)
  }
  await assert.rejects(inspectPrivilegedProcMetadata('credentials', process.pid), /unsupported metadata operation/)
})

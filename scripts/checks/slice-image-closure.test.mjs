import assert from 'node:assert/strict'
import { test } from 'node:test'
import { makeRepo } from '../lib/check-fixture.mjs'
import check from './slice-image-closure.mjs'

const DOCKER = 'apps/kernel/slice-linux-docker/docker'
const dockerfile = copies => `FROM scratch\n${copies.map(f => `COPY --chown=slice:slice ${DOCKER}/${f} /opt/chariox-slice/${f}`).join('\n')}\n`
const files = copies => ({
  [`${DOCKER}/Dockerfile`]: dockerfile(copies),
  [`${DOCKER}/host.mjs`]: 'import { a } from "./a.mjs"\nexport { a }\n',
  [`${DOCKER}/a.mjs`]: 'import { b } from "./b.mjs"\nexport const a = b\n',
  [`${DOCKER}/b.mjs`]: 'export const b = 1\n',
})

test('an import closure that the image copies passes', () => {
  const r = makeRepo(files(['host.mjs', 'a.mjs', 'b.mjs']))
  assert.equal(r.run(check.name).status, 0)
  r.dispose()
})

test('a module imported by a shipped entrypoint but not copied is red', () => {
  const r = makeRepo(files(['host.mjs', 'a.mjs', 'b.mjs']))
  r.write(`${DOCKER}/Dockerfile`, dockerfile(['host.mjs', 'a.mjs']))
  const result = r.run(check.name)
  assert.equal(result.status, 1)
  assert.match(result.text, new RegExp(`${DOCKER}/a\\.mjs:1: imports ${DOCKER}/b\\.mjs, which the slice image does not copy at /opt/chariox-slice/b\\.mjs`))
  r.dispose()
})

test('a new import added to a shipped module without a COPY line is red', () => {
  const r = makeRepo(files(['host.mjs', 'a.mjs', 'b.mjs']))
  r.write(`${DOCKER}/payment.mjs`, 'export const pay = 1\n')
  r.write(`${DOCKER}/host.mjs`, 'import { a } from "./a.mjs"\nimport { pay } from "./payment.mjs"\nexport { a, pay }\n')
  const result = r.run(check.name)
  assert.equal(result.status, 1)
  assert.match(result.text, /host\.mjs:2: imports .*payment\.mjs, which the slice image does not copy/)
  r.dispose()
})

test('a test file that is not shipped may import anything', () => {
  const r = makeRepo(files(['host.mjs', 'a.mjs', 'b.mjs']))
  r.write(`${DOCKER}/other.test.mjs`, 'import "./not-copied.mjs"\n')
  r.write(`${DOCKER}/not-copied.mjs`, 'export {}\n')
  assert.equal(r.run(check.name).status, 0)
  r.dispose()
})

// The real layout: a build stage copies the whole kernel directory, the runtime stage (the last FROM) copies the modules one by one.
const multiStage = runtimeCopies => `FROM rust:1 AS rust-builder
WORKDIR /opt/chariox-source
COPY apps/kernel apps/kernel
FROM node:22
WORKDIR /opt/chariox-toolchain
${runtimeCopies.join('\n')}
`
const copyLine = f => `COPY --chown=slice:slice ${DOCKER}/${f} /opt/chariox-slice/${f}`

test('a build stage that copies the whole directory does not ship a module the runtime stage forgot', () => {
  const r = makeRepo({ ...files([]), [`${DOCKER}/Dockerfile`]: multiStage(['host.mjs', 'a.mjs', 'b.mjs'].map(copyLine)) })
  assert.equal(r.run(check.name).status, 0)
  r.write(`${DOCKER}/Dockerfile`, multiStage(['host.mjs', 'a.mjs'].map(copyLine)))
  const result = r.run(check.name)
  assert.equal(result.status, 1)
  assert.match(result.text, new RegExp(`a\\.mjs:1: imports ${DOCKER}/b\\.mjs, which the slice image does not copy`))
  r.dispose()
})

test('a module reaches the runtime stage through COPY --from only when the build stage holds it', () => {
  const viaBuilder = path => multiStage([copyLine('host.mjs'), copyLine('a.mjs'), `COPY --from=rust-builder ${path} /opt/chariox-slice/b.mjs`])
  const r = makeRepo({ ...files([]), [`${DOCKER}/Dockerfile`]: viaBuilder(`/opt/chariox-source/${DOCKER}/b.mjs`) })
  assert.equal(r.run(check.name).status, 0)
  r.write(`${DOCKER}/Dockerfile`, viaBuilder('/opt/chariox-source/apps/relay/b.mjs'))
  assert.equal(r.run(check.name).status, 1)
  r.dispose()
})

test('a copied parent directory holds the modules only at the paths the importer resolves to', () => {
  const kernelDirectory = (...runtime) => multiStage([...runtime, 'COPY --from=rust-builder /opt/chariox-source/apps/kernel/ /opt/kernel/'])
  const r = makeRepo({ ...files([]), [`${DOCKER}/Dockerfile`]: kernelDirectory() })
  assert.equal(r.run(check.name).status, 0, 'host, a and b all sit side by side under /opt/kernel')
  // b.mjs exists in the image, but under /opt/kernel, not next to a.mjs at /opt/chariox-slice: `./b.mjs` does not find it.
  r.write(`${DOCKER}/Dockerfile`, kernelDirectory(copyLine('host.mjs'), copyLine('a.mjs')))
  const result = r.run(check.name)
  assert.equal(result.status, 1)
  assert.match(result.text, new RegExp(`a\\.mjs:1: imports ${DOCKER}/b\\.mjs, which the slice image does not copy at /opt/chariox-slice/b\\.mjs`))
  r.dispose()
})

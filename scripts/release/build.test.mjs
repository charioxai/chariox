import assert from 'node:assert/strict'
import test from 'node:test'
import { build, buildCommand, boundedDockerfile } from './build.mjs'
import { execFileSync } from 'node:child_process'
import { mkdir, mkdtemp, readFile, rm, writeFile, symlink } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

const input = { component: 'cloud-web', source: '/source', output: '/evidence/build', version: '0.1.0', revision: 'a'.repeat(40) }
test('MP-07 / MP-11: default build is a side-effect-free plan', async () => {
  const plan = await build(input)
  assert.equal(plan.mode, 'plan')
  assert.equal(plan.command, 'docker')
  assert.ok(!plan.args.includes('--push'))
  assert.ok(!plan.args.some(arg => arg.startsWith('type=registry')))
})
test('MP-07 / MP-11: publishing uses separate registry cache and immutable revision tags', () => {
  const plan = buildCommand({ ...input, mode: 'push' })
  assert.ok(plan.args.includes('--push'))
  assert.ok(!plan.args.includes('--load'))
  assert.ok(plan.args.includes('type=registry,ref=ghcr.io/charioxai/chariox-cloud-web:buildcache,mode=max'))
  assert.ok(plan.tag.endsWith(`:0.1.0-${input.revision}`))
  for (const component of ['cloud-services', 'oss', 'relay']) {
    const other = buildCommand({ ...input, component, mode: 'push' })
    assert.notEqual(other.tag, plan.tag)
  }
})
test('MP-07 / MP-11: bad revisions/modes cannot become shell or Docker inputs', () => {
  for (const change of [{ revision: 'main' }, { component: '../../other' }, { version: '0.1.0; id' }, { version: '0.1.0-01' }, { mode: 'publish' }]) {
    assert.throws(() => buildCommand({ ...input, ...change }))
  }
})
test('MP-08 / MP-09 / MP-10 / MP-11: a build memory cap does not alter the deployed runtime environment', () => {
  const source = 'FROM node:fixture AS build\nRUN node build.mjs\nFROM node:fixture AS runtime\nCMD ["node", "server.mjs"]\n'
  const result = boundedDockerfile(source)
  assert.ok(result.split('AS runtime\n')[0].includes('ENV NODE_OPTIONS=--max-old-space-size=4096'))
  assert.equal(result.split('AS runtime\n')[1], source.split('AS runtime\n')[1])
})
test('MP-08 / MP-09 / MP-10 / MP-11: branch packaging code cannot execute on the trusted runner', async () => {
  const root = await mkdtemp(join(tmpdir(), 'cicd-build-authority-'))
  const source = join(root, 'source'), output = join(root, 'output'), marker = join(root, 'branch-executed')
  const oldPath = process.env.PATH
  try {
    await mkdir(join(source, 'deploy/openship'), { recursive: true }); await mkdir(join(source, 'deploy/production'))
    await mkdir(join(root, 'bin'))
    await writeFile(join(source, 'deploy/openship/Dockerfile'), 'FROM scratch AS build\nFROM scratch AS runtime\n')
    await writeFile(join(source, 'deploy/production/cloud-control.compose.yml'), 'synthetic compose')
    await writeFile(join(source, 'deploy/openship/create-cloud-staging-release.sh'), `#!/bin/sh\nprintf branch > '${marker}'\nprintf synthetic > "$3"\n`)
    await writeFile(join(source, 'deploy/openship/release-source-digest.mjs'), `console.log('${'a'.repeat(64)}')\n`)
    const git = args => execFileSync('git', ['-c', 'user.name=Fixture', '-c', 'user.email=fixture@example.test', '-c', 'commit.gpgsign=false', ...args], { cwd: source, encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] }).trim()
    git(['init', '-q']); git(['add', '.']); git(['commit', '-qm', 'Fixture input [skip ci]'])
    const revision = git(['rev-parse', 'HEAD'])
    await writeFile(join(root, 'bin/docker'), `#!${process.execPath}\nconst fs=require('node:fs'),a=process.argv.slice(2);fs.writeFileSync(a[a.indexOf('--metadata-file')+1], JSON.stringify({'containerimage.digest':'sha256:${'a'.repeat(64)}'}));\n`, { mode: 0o755 })
    process.env.PATH = `${join(root, 'bin')}:${oldPath}`
    await assert.rejects(() => build({ component: 'cloud-web', source, output, revision, version: '0.1.0', mode: 'local' }), /trusted Cloud main/)
    await assert.rejects(readFile(marker), { code: 'ENOENT' })
    const trusted = join(root, 'trusted-main')
    execFileSync('git', ['clone', '-q', '--shared', source, trusted], { stdio: 'pipe' })
    await writeFile(join(trusted, 'deploy/openship/create-cloud-staging-release.sh'), '#!/bin/sh\nprintf trusted-artifact > "$3"\n')
    const trustedGit = args => execFileSync('git', ['-c', 'user.name=Fixture', '-c', 'user.email=fixture@example.test', '-c', 'commit.gpgsign=false', ...args], { cwd: trusted, encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] }).trim()
    trustedGit(['add', '.']); trustedGit(['commit', '-qm', 'Trusted packaging fixture [skip ci]'])
    const workflowRevision = trustedGit(['rev-parse', 'HEAD'])
    await build({ component: 'cloud-web', source, output, revision, version: '0.1.0', mode: 'local', cloudTools: trusted, workflowRevision })
    assert.equal(await readFile(join(output, 'cloud.tar.gz'), 'utf8'), 'trusted-artifact')
    await assert.rejects(readFile(marker), { code: 'ENOENT' })
    await rm(join(source, 'deploy/openship/Dockerfile'))
    await writeFile(join(root, 'outside-context'), 'synthetic sentinel outside the source archive')
    await symlink(join(root, 'outside-context'), join(source, 'deploy/openship/Dockerfile'))
    git(['add', '.']); git(['commit', '-qm', 'Linked source fixture [skip ci]'])
    await assert.rejects(() => build({ component: 'cloud-web', source, output: join(root, 'linked-output'), revision: git(['rev-parse', 'HEAD']),
      version: '0.1.0', mode: 'local', cloudTools: trusted, workflowRevision }), /linked or special/)
  } finally { process.env.PATH = oldPath; await rm(root, { recursive: true, force: true }) }
})

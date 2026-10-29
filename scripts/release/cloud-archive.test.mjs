import assert from 'node:assert/strict'
import { execFileSync } from 'node:child_process'
import { copyFile, mkdir, mkdtemp, readFile, rm, symlink, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import test from 'node:test'
import { fileURLToPath } from 'node:url'
import { build } from './build.mjs'

const fixtures = new URL('./fixtures/cloud-archive/', import.meta.url)
const git = (cwd, args) => execFileSync('git', ['-c', 'user.name=Fixture', '-c', 'user.email=fixture@example.test',
  '-c', 'commit.gpgsign=false', ...args], { cwd, encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] }).trim()

async function setup(root, helper) {
  const source = join(root, 'source'), trusted = join(root, 'trusted-main')
  const marker = join(root, 'branch-executed'), dockerMarker = join(root, 'docker-args.json')
  for (const directory of [source, trusted]) {
    await mkdir(join(directory, 'deploy/openship'), { recursive: true })
    git(directory, ['init', '-q'])
  }
  await mkdir(join(source, 'deploy/production'))
  for (const path of ['deploy/production/cloud-control.compose.yml', 'deploy/production/publication-services.compose.yml',
    'deploy/openship/publication-services.compose.yml']) await writeFile(join(source, path), 'synthetic compose\n')
  for (const suffix of ['', '.publication-services']) {
    await writeFile(join(source, `deploy/openship/Dockerfile${suffix}`), 'FROM scratch AS build\nFROM scratch AS runtime\n')
  }
  const branchDigest = `import { writeFileSync } from 'node:fs'\nwriteFileSync(${JSON.stringify(marker)}, 'branch digest executed')\nconsole.log('${'b'.repeat(64)}')\n`
  await writeFile(join(source, 'deploy/openship/release-source-digest.mjs'), branchDigest)
  await writeFile(join(source, 'deploy/openship/create-cloud-staging-release.sh'), `#!/bin/sh\nprintf branch > '${marker}'\nexit 1\n`)
  git(source, ['add', '.']); git(source, ['commit', '-qm', 'Selected source fixture [skip ci]'])
  await copyFile(new URL(helper, fixtures), join(trusted, 'deploy/openship/create-cloud-staging-release.sh'))
  for (const name of ['release-source-digest.mjs', 'extract-cloud-staging-release.py']) {
    await copyFile(new URL(name, fixtures), join(trusted, 'deploy/openship', name))
  }
  git(trusted, ['add', '.']); git(trusted, ['commit', '-qm', 'Trusted tools fixture [skip ci]'])
  await mkdir(join(root, 'bin'))
  await writeFile(join(root, 'bin/docker'), `#!${process.execPath}\nconst fs=require('node:fs'),a=process.argv.slice(2);fs.writeFileSync(${JSON.stringify(dockerMarker)}, JSON.stringify(a));fs.writeFileSync(a[a.indexOf('--metadata-file')+1], JSON.stringify({'containerimage.digest':'sha256:${'a'.repeat(64)}'}));\n`, { mode: 0o755 })
  return { source, trusted, marker, dockerMarker, branchDigest, revision: git(source, ['rev-parse', 'HEAD']),
    workflowRevision: git(trusted, ['rev-parse', 'HEAD']) }
}

test('MP-08 / MP-09 / MP-10 / MP-11: CLI explains rejected archive tooling in one line', async () => {
  await withFixture('main-create.sh', async ({ root, source, trusted, marker, dockerMarker, revision, workflowRevision }) => {
    const config = join(root, 'build-input.json')
    await writeFile(config, JSON.stringify({ component: 'cloud-web', source, output: join(root, 'output'), revision,
      version: '0.1.0', mode: 'local', cloudTools: trusted, workflowRevision }))
    assert.throws(() => execFileSync(process.execPath, [fileURLToPath(new URL('./build.mjs', import.meta.url)), config],
      { encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] }), error => {
      assert.equal(error.status, 1)
      assert.equal(error.stdout, '')
      assert.equal(error.stderr, 'MP-08 / MP-09 / MP-10 / MP-11: unsupported Cloud archive helper; pin Cloud tooling containing the reviewed trusted digest helper\n')
      return true
    })
    await assert.rejects(readFile(marker), { code: 'ENOENT' })
    await assert.rejects(readFile(dockerMarker), { code: 'ENOENT' })
  })
})

async function withFixture(helper, body) {
  const root = await mkdtemp(join(tmpdir(), 'cicd-real-archive-'))
  const oldPath = process.env.PATH
  try {
    const fixture = await setup(root, helper)
    process.env.PATH = `${join(root, 'bin')}:${oldPath}`
    await body({ ...fixture, root })
  } finally {
    process.env.PATH = oldPath
    await rm(root, { recursive: true, force: true })
  }
}

for (const component of ['cloud-web', 'cloud-services']) {
  test(`MP-08 / MP-09 / MP-10 / MP-11: ${component} rejects real Cloud main's transitive source execution`, async () => {
    await withFixture('main-create.sh', async ({ root, source, trusted, marker, dockerMarker, revision, workflowRevision }) => {
      let failure
      const output = join(root, 'output')
      try {
        await build({ component, source, output, revision, version: '0.1.0', mode: 'local', cloudTools: trusted, workflowRevision })
      } catch (error) { failure = error }
      assert.equal(await readFile(marker, 'utf8').catch(error => {
        if (error.code === 'ENOENT') return null
        throw error
      }), null, 'selected-source digest executed on the trusted host')
      assert.equal(failure?.code, 'ERR_UNSUPPORTED_CLOUD_ARCHIVE_HELPER')
      assert.match(failure.message, /pin Cloud tooling containing the reviewed trusted digest helper/)
      await assert.rejects(readFile(dockerMarker), { code: 'ENOENT' })
      await assert.rejects(readFile(join(output, 'receipt.json')), { code: 'ENOENT' })
    })
  })

  test(`MP-08 / MP-09 / MP-10 / MP-11: ${component} runs the real promotion archive chain with trusted digest code`, async () => {
    await withFixture('promotion-create.sh', async ({ root, source, trusted, marker, dockerMarker, branchDigest, revision, workflowRevision }) => {
      const options = { component, source, output: join(root, 'output'), revision, version: '0.1.0', mode: 'local' }
      await assert.rejects(() => build(options), /trusted Cloud main/)
      const facts = await build({ ...options, cloudTools: trusted, workflowRevision })
      const digest = execFileSync(process.execPath, [join(trusted, 'deploy/openship/release-source-digest.mjs'), source], { encoding: 'utf8' }).trim()
      const archiveFile = path => execFileSync('tar', ['-xOf', join(options.output, 'cloud.tar.gz'), `./${path}`], { encoding: 'utf8' })
      assert.equal(facts.source_sha256, digest)
      assert.equal(facts.packaging_revision, workflowRevision)
      assert.equal(archiveFile('.chariox-release-source-sha256').trim(), digest)
      assert.equal(archiveFile('.chariox-release-revision').trim(), revision)
      assert.equal(archiveFile('deploy/openship/release-source-digest.mjs'), branchDigest)
      const args = JSON.parse(await readFile(dockerMarker, 'utf8'))
      assert.ok(args.includes('--load'))
      assert.ok(!args.includes('--push'))
      if (component === 'cloud-services') assert.ok(args.includes(`CHARIOX_RELEASE_SOURCE_SHA256=${digest}`))
      await assert.rejects(readFile(marker), { code: 'ENOENT' })
      await rm(join(source, 'deploy/openship/Dockerfile'))
      await writeFile(join(root, 'outside-context'), 'sentinel outside source archive')
      await symlink(join(root, 'outside-context'), join(source, 'deploy/openship/Dockerfile'))
      git(source, ['add', '.']); git(source, ['commit', '-qm', 'Linked source fixture [skip ci]'])
      await assert.rejects(() => build({ ...options, output: join(root, 'linked-output'), revision: git(source, ['rev-parse', 'HEAD']),
        cloudTools: trusted, workflowRevision }), /linked or special/)
    })
  })
}

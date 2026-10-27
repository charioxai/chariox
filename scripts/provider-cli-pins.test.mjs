// MP-08/MP-11: the managed policy is shared by headed slices and publication.
import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import test from 'node:test'

const root = new URL('../', import.meta.url)
const policy = Object.fromEntries((await readFile(new URL('deploy/managed-kernel/provider-versions.env', root), 'utf8'))
  .trim().split('\n').map(line => line.split('=')))
const providers = {
  '@openai/codex': 'CHARIOX_CODEX_VERSION',
  'opencode-ai': 'CHARIOX_OPENCODE_VERSION',
  '@anthropic-ai/claude-code': 'CHARIOX_CLAUDE_VERSION',
}
assert.deepEqual(Object.keys(policy).sort(), Object.values(providers).sort())
for (const version of Object.values(policy)) assert.match(version, /^\d+\.\d+\.\d+$/)

for (const [name, directory, dockerfilePath] of [
  ['headed slice', 'apps/kernel/slice-linux-docker', 'apps/kernel/slice-linux-docker/docker/Dockerfile'],
  ['publication', 'docker/publication', 'docker/publication/Dockerfile'],
]) {
  test(`${name} provider package, lock and build checks match the shared policy`, async () => {
    const manifest = JSON.parse(await readFile(new URL(`${directory}/toolchain/package.json`, root), 'utf8'))
    const lock = JSON.parse(await readFile(new URL(`${directory}/toolchain/package-lock.json`, root), 'utf8'))
    for (const [pkg, key] of Object.entries(providers)) {
      const expected = policy[key]
      assert.equal(manifest.dependencies[pkg], expected, `${name} ${pkg} manifest drift`)
      assert.equal(lock.packages[''].dependencies[pkg], expected, `${name} ${pkg} lock root drift`)
      assert.equal(lock.packages[`node_modules/${pkg}`].version, expected, `${name} ${pkg} installed lock drift`)
    }
    const dockerfile = await readFile(new URL(dockerfilePath, root), 'utf8')
    assert.match(dockerfile, /COPY deploy\/managed-kernel\/provider-versions\.env \.\//)
    assert.match(dockerfile, /\. \.\/provider-versions\.env/)
    for (const [cli, key, prefix, suffix] of [
      ['codex', 'CHARIOX_CODEX_VERSION', 'codex-cli ', ''],
      ['opencode', 'CHARIOX_OPENCODE_VERSION', '', ''],
      ['claude', 'CHARIOX_CLAUDE_VERSION', '', ' (Claude Code)'],
    ]) {
      assert.ok(dockerfile.includes(`test "$(node_modules/.bin/${cli} --version)" = "${prefix}\${${key}}${suffix}"`), `${name} ${cli} build must verify the policy version`)
    }
    if (name === 'publication') {
      for (const [key, version] of Object.entries(policy)) {
        assert.ok(dockerfile.includes(`ARG ${key}=${version}\n`), `publication ${key} label default drift`)
      }
    }
  })
}

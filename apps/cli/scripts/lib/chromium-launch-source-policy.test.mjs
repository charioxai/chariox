import assert from 'node:assert/strict'
import { execFileSync } from 'node:child_process'
import { readFile } from 'node:fs/promises'
import path from 'node:path'
import test from 'node:test'
import { fileURLToPath } from 'node:url'

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../../../..')
const forbidden = [
  '--no-sandbox',
  '--disable-setuid-sandbox',
  '--disable-seccomp-filter-sandbox',
  '--disable-namespace-sandbox',
  '--unsafely-treat-insecure-origin-as-secure',
]

function isEvidenceOnly(relativePath) {
  const basename = path.basename(relativePath)
  return relativePath.startsWith('docs/')
    || relativePath.includes('/fixtures/')
    || relativePath.includes('/testdata/')
    || relativePath.includes('/tests/')
    || /^(?:test_|tests\.)/.test(basename)
    || /(?:^|\.)(?:test|spec|browser-test)\.[cm]?[jt]sx?$/.test(basename)
    || relativePath === 'apps/kernel/slice-linux-docker/slice-browser-profile.drill.mjs'
}

function forbiddenLauncherArguments(relativePath, source) {
  if (isEvidenceOnly(relativePath)) return []
  return source.split(/\r?\n/).flatMap((line, index) => forbidden.flatMap((flag) => {
    if (!line.includes(flag)) return []
    const evidenceCheck = new RegExp(`\\.(?:includes|startsWith)\\s*\\(\\s*(['\"\`])${flag}\\1`, 'g')
    if (!line.replace(evidenceCheck, '').includes(flag)) return []
    return [`${relativePath}:${index + 1}: ${flag}`]
  }))
}

test('source policy distinguishes evidence strings from executable browser arguments', () => {
  assert.deepEqual(
    forbiddenLauncherArguments('apps/example/live-launch.mjs', "spawn('chromium', ['--no-sandbox'])"),
    ['apps/example/live-launch.mjs:1: --no-sandbox'],
  )
  assert.deepEqual(
    forbiddenLauncherArguments('apps/example/live-launch.mjs', "args.some(arg => arg.startsWith('--no-sandbox'))"),
    [],
  )
  assert.deepEqual(
    forbiddenLauncherArguments('apps/example/live-launch.test.mjs', "spawn('chromium', ['--no-sandbox'])"),
    [],
  )
  assert.deepEqual(
    forbiddenLauncherArguments('docs/browser.md', '`--no-sandbox` is forbidden'),
    [],
  )
})

test('no tracked live or product launcher supplies a Chromium sandbox bypass', async () => {
  const tracked = execFileSync('git', ['ls-files', '-z'], { cwd: repoRoot })
    .toString('utf8')
    .split('\0')
    .filter((relativePath) => /\.(?:mjs|cjs|js|ts|tsx|sh|bash|py|rs)$/.test(relativePath))
  const findings = []
  for (const relativePath of tracked) {
    const source = await readFile(path.join(repoRoot, relativePath), 'utf8')
    findings.push(...forbiddenLauncherArguments(relativePath, source))
  }
  assert.deepEqual(findings, [])
})

test('affected live launchers invoke the shared sandbox preflight and production container policy', async () => {
  const x11Launchers = [
    'apps/cli/scripts/live-computer-secret-input-x11-drill.mjs',
    'apps/cli/scripts/live-computer-keyboard-x11-drill.mjs',
    'apps/cli/scripts/live-computer-clipboard-x11-drill.mjs',
  ]
  for (const relativePath of x11Launchers) {
    const source = await readFile(path.join(repoRoot, relativePath), 'utf8')
    assert.match(source, /chromium-sandbox-preflight\.mjs/, `${relativePath} omits sandbox preflight`)
    assert.match(source, /chromium-seccomp\.json/, `${relativePath} omits production seccomp policy`)
    assert.match(source, /XDG_RUNTIME_DIR=/, `${relativePath} omits the private browser runtime`)
  }
  const cloud = await readFile(
    path.join(repoRoot, 'apps/cli/scripts/lib/live-cloud-publication-deployment-drill-transport.mjs'),
    'utf8',
  )
  assert.equal(cloud.match(/await preflightChromiumSandbox\(/g)?.length, 2)
  assert.equal(cloud.match(/runtimeDirectory: runtimeDir/g)?.length, 2)
})

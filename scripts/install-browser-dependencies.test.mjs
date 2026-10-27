// MP-08/MP-10/MP-11: the installer must give the pinned kernel Chromium its shared libraries.
import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import { chmod, mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises'
import { existsSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, join, resolve } from 'node:path'
import { test } from 'node:test'
import { fileURLToPath } from 'node:url'

const script = resolve(dirname(fileURLToPath(import.meta.url)), '../deploy/local-linux/install-browser-dependencies.sh')
const sonames = ['libasound.so.2', 'libatk-1.0.so.0', 'libatk-bridge-2.0.so.0', 'libatspi.so.0', 'libcups.so.2', 'libgbm.so.1', 'libnspr4.so', 'libnss3.so', 'libxkbcommon.so.0', 'libXcomposite.so.1', 'libXdamage.so.1', 'libXrandr.so.2', 'libX11.so.6', 'libXext.so.6', 'libXtst.so.6']
const t64 = ['libasound2', 'libatk-bridge2.0-0', 'libatk1.0-0', 'libatspi2.0-0', 'libcups2', 'libglib2.0-0']

async function fixture({ desktop, libraries, missing }) {
  const root = await mkdtemp(join(tmpdir(), 'chariox-browser-deps-'))
  const bin = join(root, 'usr/bin')
  await mkdir(bin, { recursive: true })
  const put = async (path, body) => { await writeFile(path, body); await chmod(path, 0o755) }
  const state = join(root, 'libraries-installed'), calls = join(root, 'apt-calls'), updated = join(root, 'apt-updated')
  if (desktop) for (const name of ['Xvfb', 'openbox', 'bwrap', 'dbus-daemon', 'setxkbmap']) await put(join(bin, name), '#!/bin/sh\n')
  if (libraries) await writeFile(state, '')
  await put(join(bin, 'python3'), '#!/bin/sh\nexit 0\n')
  // A fresh image has empty package lists: apt-cache learns the archive only after `apt-get update`.
  await put(join(bin, 'apt-cache'), `#!/bin/sh\n[ -e '${updated}' ] || exit 100\ncase "$2" in ${t64.map(name => `${name}t64`).join('|')}) exit 0 ;; *) exit 100 ;; esac\n`)
  await put(join(bin, 'apt-get'), `#!/bin/sh
echo "$@" >> '${calls}'
[ "$1" = update ] && : > '${updated}'
if [ "$1" = install ]; then
  for name in Xvfb openbox bwrap dbus-daemon setxkbmap; do : > '${bin}'/$name; chmod 755 '${bin}'/$name; done
  : > '${state}'
fi
`)
  await put(join(root, 'ldconfig'), `#!/bin/sh\n[ -e '${state}' ] && printf '%s\\n' ${sonames.filter(name => name !== missing).map(name => `'\t${name} (libc6,x86-64) => /usr/lib/${name}'`).join(' ')}\nexit 0\n`)
  return { root, calls, ldconfig: join(root, 'ldconfig') }
}
const run = f => spawnSync('bash', [script], { encoding: 'utf8', env: { PATH: '/usr/bin:/bin', CHARIOX_LOCAL_INSTALL_ROOT: f.root, CHARIOX_LOCAL_INSTALL_LDCONFIG: f.ldconfig } })

test('a fresh server image with the desktop tools but no Chromium libraries and empty APT lists installs them (t64 names resolved after update)', async () => {
  const f = await fixture({ desktop: true, libraries: false })
  try {
    const result = run(f)
    assert.equal(result.status, 0, result.stderr)
    assert(existsSync(f.calls), 'apt was never invoked although Chromium cannot load its libraries')
    const install = (await readFile(f.calls, 'utf8')).split('\n').find(line => line.startsWith('install'))
    for (const name of ['libasound2t64', 'libatk-bridge2.0-0t64', 'libatk1.0-0t64', 'libatspi2.0-0t64', 'libcups2t64', 'libglib2.0-0t64', 'libxcomposite1', 'libxdamage1', 'libnss3', 'libxtst6', 'python3-gi', 'python3-pyatspi']) assert(install.split(' ').includes(name), name)
  } finally { await rm(f.root, { recursive: true, force: true }) }
})

test('nothing is installed when the desktop tools and Chromium libraries are already present', async () => {
  const f = await fixture({ desktop: true, libraries: true })
  try {
    const result = run(f)
    assert.equal(result.status, 0, result.stderr)
    assert.match(result.stdout, /unchanged system browser dependencies/)
    assert(!existsSync(f.calls))
  } finally { await rm(f.root, { recursive: true, force: true }) }
})

test('a bare image installs both the desktop tools and the libraries', async () => {
  const f = await fixture({ desktop: false, libraries: false })
  try {
    const result = run(f)
    assert.equal(result.status, 0, result.stderr)
    const install = (await readFile(f.calls, 'utf8')).split('\n').find(line => line.startsWith('install'))
    assert(install.includes('xvfb') && install.includes('libasound2t64'))
  } finally { await rm(f.root, { recursive: true, force: true }) }
})

// MP-08/MP-10: old installations can have every Chromium dependency except Xtst.
test('an existing Chromium installation without libXtst installs the native display libraries', async () => {
  const f = await fixture({ desktop: true, libraries: true, missing: 'libXtst.so.6' })
  try {
    const result = run(f)
    assert(existsSync(f.calls), 'the missing native display dependency must trigger installation')
    const calls = await readFile(f.calls, 'utf8')
    assert.match(calls, /libxtst6/)
    // This stub deliberately keeps the library missing after apt completes.
    assert.equal(result.status, 1)
    assert.match(result.stderr, /dependencies still missing/)
  } finally { await rm(f.root, { recursive: true, force: true }) }
})

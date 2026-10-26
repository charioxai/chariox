// MP-07 / MP-11: unsigned inputs; release signing stays on the owner's Mac.
import { copyFile, mkdir, readFile, writeFile } from 'node:fs/promises'
import { join, resolve } from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'
import { canonical, isVersion } from './manifest.mjs'
import { hashFile } from './build.mjs'
import { resourceGuard, run } from './process.mjs'

export async function nativeBuild({ source, output, version, revision, slot }) {
  source = resolve(source); output = resolve(output)
  if (!/^[0-9a-f]{40}$/.test(revision) || !isVersion(version)) throw Error('exact revision and product version required')
  if (!slot?.startsWith('/')) throw Error('absolute coordinator Cargo-slot wrapper required')
  if (run('git', ['rev-parse', 'HEAD'], { cwd: source }) !== revision || run('git', ['status', '--porcelain'], { cwd: source })) throw Error('native source must be clean and at exact revision')
  await resourceGuard(source)
  await mkdir(output, { recursive: false })
  const env = { ...process.env, CARGO_BUILD_JOBS: '4', NODE_OPTIONS: '--max-old-space-size=4096',
    CHARIOX_RELEASE_SOURCE: source, CHARIOX_RELEASE_OUTPUT: output }
  delete env.CARGO_TARGET_DIR
  // Both compilation and copying run while the slot is held; no target globs.
  run(slot, ['node', fileURLToPath(new URL('./native-slot.mjs', import.meta.url))], { cwd: source, env, log: join(output, 'cargo.log'), timeout: 3 * 60 * 60_000 })
  run('pnpm', ['install', '--frozen-lockfile'], { cwd: source, env, log: join(output, 'cli.log') })
  run('pnpm', ['--filter', '@chariox/cli', 'run', 'build'], { cwd: source, env, log: join(output, 'cli.log') })
  run('bun', ['apps/cli/scripts/compile.mjs', '--target', 'linux-x64', '--outfile', join(output, 'chariox'), '--version', version], { cwd: source, env, log: join(output, 'cli.log') })
  const names = ['chariox', 'chariox-kernel', 'chariox-relay']
  const files = Object.fromEntries(await Promise.all(names.map(async name => [name, await hashFile(join(output, name))])))
  await writeFile(join(output, 'version.json'), canonical({ version, revision, files }))
  const epoch = run('git', ['log', '-1', '--format=%ct'], { cwd: source })
  run('tar', ['--sort=name', `--mtime=@${epoch}`, '--owner=0', '--group=0', '--numeric-owner', '-czf', join(output, 'native.tar.gz'), '-C', output, ...names, 'version.json'])
  const receipt = { version, revision, files, bundle_sha256: await hashFile(join(output, 'native.tar.gz')), relay_sha256: files['chariox-relay'], signing: 'pending' }
  await writeFile(join(output, 'native.json'), canonical(receipt), { flag: 'wx' })
  return receipt
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  readFile(process.argv[2], 'utf8').then(JSON.parse).then(nativeBuild).then(result => console.log(canonical(result)))
    .catch(() => { console.error('MP-07 / MP-11: unsigned native build failed; inspect lane-owned cargo.log/cli.log'); process.exitCode = 1 })
}

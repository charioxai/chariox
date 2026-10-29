// MP-07 / MP-11. Plan by default; exact Git archives only, no worktree COPY.
import { copyFile, mkdir, readFile, readdir, rm, writeFile } from 'node:fs/promises'
import { join, resolve } from 'node:path'
import { pathToFileURL, fileURLToPath } from 'node:url'
import { canonical, hash, isVersion } from './manifest.mjs'
import { resourceGuard, run } from './process.mjs'

const toolsRoot = fileURLToPath(new URL('../..', import.meta.url))
const components = ['cloud-web', 'cloud-services', 'oss', 'relay']
const repositories = { 'cloud-web': 'chariox-cloud-web', 'cloud-services': 'chariox-cloud-services', oss: 'chariox-oss', relay: 'chariox-relay' }

export function boundedDockerfile(source) {
  return source.replace(/^(FROM[^\n]+\sAS\s+build)\s*$/gmi, '$1\nENV NODE_OPTIONS=--max-old-space-size=4096')
}

export function buildCommand({ component, source, output, version, revision, mode = 'plan' }) {
  if (!components.includes(component) || !['plan', 'local', 'push'].includes(mode)) throw Error('invalid component/mode')
  if (!/^[0-9a-f]{40}$/.test(revision) || !isVersion(version)) throw Error('exact revision and product version required')
  const context = join(resolve(output), 'context')
  if (resolve(output).startsWith(`${resolve(source)}/`) || resolve(output) === resolve(source)) throw Error('build state must be outside the source checkout')
  const tag = mode === 'local' ? `chariox-release-local/${component}:${revision}-${hash(resolve(output)).slice(0, 12)}` : `ghcr.io/charioxai/${repositories[component]}:${version}-${revision}`
  const args = ['buildx', 'build', '--platform', 'linux/amd64', '--provenance=mode=min', '--sbom=true',
    '--file', join(resolve(output), 'Dockerfile'), '--tag', tag,
    '--label', `org.opencontainers.image.version=${version}`, '--label', `org.opencontainers.image.revision=${revision}`,
    '--metadata-file', join(resolve(output), 'metadata.json')]
  if (mode === 'push') args.push('--cache-from', `type=registry,ref=ghcr.io/charioxai/${repositories[component]}:buildcache`,
    '--cache-to', `type=registry,ref=ghcr.io/charioxai/${repositories[component]}:buildcache,mode=max`, '--push')
  else args.push('--load')
  args.push(context)
  return { component, revision, version, source: resolve(source), output: resolve(output), mode, tag, command: 'docker', args }
}

async function rejectPrivateFiles(directory) {
  for (const entry of await readdir(directory, { withFileTypes: true })) {
    if (entry.isSymbolicLink() || !entry.isFile() && !entry.isDirectory()) throw Error('linked or special entry in exact source archive')
    if (entry.isDirectory()) await rejectPrivateFiles(join(directory, entry.name))
    if ((/^\.env(?:\.|$)/.test(entry.name) && entry.name !== '.env.example') ||
      ['auth.json', '.credentials.json', 'id_rsa', 'id_ed25519'].includes(entry.name)) throw Error('private filename in exact source archive')
  }
}

export async function build(options) {
  const plan = buildCommand(options)
  if (plan.mode === 'plan') return plan
  if (plan.component.startsWith('cloud-') && (!options.cloudTools?.startsWith('/') || !/^[0-9a-f]{40}$/.test(options.workflowRevision))) throw Error('trusted Cloud main tooling and exact workflow revision required')
  if (plan.component.startsWith('cloud-') && (run('git', ['rev-parse', 'HEAD'], { cwd: options.cloudTools }) !== options.workflowRevision ||
      run('git', ['status', '--porcelain'], { cwd: options.cloudTools }))) throw Error('trusted Cloud main tooling revision mismatch or dirty checkout')
  await resourceGuard(plan.source)
  const { component, revision, version, source, output } = plan
  if (run('git', ['rev-parse', `${revision}^{commit}`], { cwd: source }) !== revision) throw Error('source commit missing')
  await mkdir(output, { recursive: false })
  const context = join(output, 'context'), log = join(output, 'build.log')
  await mkdir(context)
  const facts = { component, revision, version, mode: plan.mode }
  try {
    if (component.startsWith('cloud-')) {
      facts.packaging_revision = options.workflowRevision
      const tar = join(output, 'source.tar')
      run('git', ['archive', '--format=tar', `--output=${tar}`, revision], { cwd: source })
      run('tar', ['-xf', tar, '-C', context])
      await rm(tar)
      await rejectPrivateFiles(context)
      const relative = `deploy/openship/Dockerfile${component === 'cloud-services' ? '.publication-services' : ''}`
      const dockerfile = await readFile(join(context, relative), 'utf8')
      // Bound every Node build process without modifying declared source bytes.
      await writeFile(join(output, 'Dockerfile'), boundedDockerfile(dockerfile))
      facts.source_dockerfile_sha256 = hash(dockerfile)
      facts.build_dockerfile_sha256 = await hashFile(join(output, 'Dockerfile'))
      // Prevent .dockerignore from changing the exact Git archive source.
      await writeFile(join(output, 'Dockerfile.dockerignore'), '# Exact Git archive, no untracked files.\n')
      const digest = path => hashFile(join(context, path))
      const production = await digest(component === 'cloud-web' ? 'deploy/production/cloud-control.compose.yml' : 'deploy/production/publication-services.compose.yml')
      const extra = ['--build-arg', `CHARIOX_CLOUD_REVISION=${revision}`, '--build-arg', `CHARIOX_PRODUCTION_COMPOSE_SHA256=${production}`]
      const archive = join(output, 'cloud.tar.gz')
      run('sh', [join(options.cloudTools, 'deploy/openship/create-cloud-staging-release.sh'), source, revision, archive], { log })
      facts.archive_sha256 = await hashFile(archive)
      facts.source_sha256 = run('node', [join(options.cloudTools, 'deploy/openship/release-source-digest.mjs'), context])
      if (component === 'cloud-services') extra.push('--build-arg', `CHARIOX_STAGING_COMPOSE_SHA256=${await digest('deploy/openship/publication-services.compose.yml')}`,
        '--build-arg', `CHARIOX_RELEASE_SOURCE_SHA256=${facts.source_sha256}`)
      plan.args.splice(plan.args.length - 1, 0, ...extra)
    } else {
      if (!options.native) throw Error('native build directory required')
      const native = resolve(options.native)
      const receipt = JSON.parse(await readFile(join(native, 'native.json'), 'utf8'))
      if (receipt.revision !== revision || receipt.version !== version) throw Error('native build identity mismatch')
      const name = component === 'relay' ? 'chariox-relay' : 'native.tar.gz'
      const expected = component === 'relay' ? receipt.relay_sha256 : receipt.bundle_sha256
      if (await hashFile(join(native, name)) !== expected) throw Error('native artifact checksum mismatch')
      await copyFile(join(native, name), join(context, name))
      await copyFile(join(native, 'native.json'), join(context, 'native.json'))
      if (receipt.signing === 'signed') {
        await copyFile(join(native, 'native.json.sig'), join(context, 'native.json.sig'))
        await copyFile(join(native, 'release-public.pem'), join(context, 'release-public.pem'))
      }
      // Artifact carrier, not a replacement for MP-07 signed native activation.
      await copyFile(join(toolsRoot, 'deploy/release/Dockerfile.artifact'), join(output, 'Dockerfile'))
      Object.assign(facts, { bundle_sha256: receipt.bundle_sha256, binary_sha256: receipt.relay_sha256 })
    }
    run('docker', plan.args, { log, timeout: 2 * 60 * 60_000 })
    const metadata = JSON.parse(await readFile(join(output, 'metadata.json'), 'utf8'))
    const digest = metadata['containerimage.digest']
    if (!/^sha256:[0-9a-f]{64}$/.test(digest)) throw Error('Buildx omitted image manifest digest')
    // A --load config ID is never represented as a deployable registry digest.
    facts.image = plan.mode === 'push' ? `${plan.tag.split(':')[0]}@${digest}` : null
    facts.local_image_digest = digest
    facts.local_tag = plan.mode === 'local' ? plan.tag : null
    await writeFile(join(output, 'receipt.json'), canonical(facts), { flag: 'wx' })
    return facts
  } finally {
    // Only the exact archive context this invocation created, never caches/images.
    await rm(context, { recursive: true, force: true })
  }
}

export async function hashFile(path) { return hash(await readFile(path)) }

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  readFile(process.argv[2], 'utf8').then(JSON.parse).then(build).then(result => console.log(canonical(result)))
    .catch(() => { console.error('MP-07 / MP-11: build failed; inspect lane-owned build.log (no runtime secrets accepted)'); process.exitCode = 1 })
}

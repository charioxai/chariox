// Every module a slice entrypoint imports must be copied into the slice image, or the slice host fails at start with a missing module.
import { posix } from 'node:path'

const DOCKERFILE = 'apps/kernel/slice-linux-docker/docker/Dockerfile'
const ENTRY = /^apps\/kernel\/slice-linux-docker\/docker\/[^/]+\.mjs$/
const NOT_SHIPPED = /\.(test|browser-test)\.mjs$/
const IMPORT = /(?:\bfrom\s*|\bimport\s*\(?\s*|\brequire\(\s*)['"](\.{1,2}\/[^'"]+)['"]/g

// What the final (runtime) stage holds, as {dest, src} pairs: `dest` is a path in the image, `src` the repository path it was copied from.
// Earlier stages only count when the runtime stage copies from them (COPY --from=<stage>), so a build stage that copies a whole
// directory does not make the runtime image hold it.
function runtimeContents(dockerfile, files) {
  const stages = new Map()
  let current = null
  const isDirectory = src => files.some(f => f.startsWith(`${src}/`))
  for (const line of dockerfile.replace(/\\\n/g, ' ').split('\n')) {
    const from = /^\s*FROM\s+(?:--\S+\s+)*(\S+)(?:\s+AS\s+(\S+))?/i.exec(line)
    if (from) {
      const parent = stages.get(from[1])
      current = { name: from[2] ?? `#${stages.size}`, workdir: '/', entries: parent ? parent.entries.map(e => ({ ...e })) : [] }
      stages.set(current.name, current)
      continue
    }
    const workdir = /^\s*WORKDIR\s+(\S+)/i.exec(line)
    if (workdir && current) { current.workdir = posix.resolve(current.workdir, workdir[1]); continue }
    const copy = /^\s*COPY\s+(.*)$/i.exec(line)
    if (!copy || !current) continue
    const words = copy[1].trim().split(/\s+/)
    const source = words.find(w => w.startsWith('--from='))?.slice('--from='.length)
    const paths = words.filter(w => !w.startsWith('--'))
    const dest = paths.pop()
    const toDirectory = paths.length > 1 || dest.endsWith('/')
    const target = posix.resolve(current.workdir, dest)
    for (const raw of paths) {
      const src = posix.normalize(raw).replace(/\/$/, '')
      // A directory copies its contents into the destination; a file lands under it when the destination is a directory.
      if (source === undefined) {
        current.entries.push({ dest: toDirectory && !isDirectory(src) ? posix.join(target, posix.basename(src)) : target, src })
        continue
      }
      for (const e of stages.get(source)?.entries ?? []) {
        const path = posix.resolve(stages.get(source).workdir, raw)
        // A path inside a copied directory maps to the same place inside its source; a copied parent holds the whole entry.
        if (path === e.dest || path.startsWith(`${e.dest}/`)) {
          const from = posix.join(e.src, posix.relative(e.dest, path))
          current.entries.push({ dest: isDirectory(from) || !toDirectory ? target : posix.join(target, posix.basename(path)), src: from })
        } else if (e.dest.startsWith(`${path}/`)) current.entries.push({ dest: posix.join(target, posix.relative(path, e.dest)), src: e.src })
      }
    }
  }
  return current?.entries ?? []
}

export default {
  name: 'slice-image-closure',
  why: 'present before this check existed',
  rule: 'Review class: a module imported by the slice host but not shipped in the slice image',
  rootFix: `add a COPY line for the module to ${DOCKERFILE} next to its importer (or stop importing it from the slice host)`,
  scan(ctx) {
    const dockerfile = ctx.read(DOCKERFILE)
    if (dockerfile === null) return []
    const tracked = new Set(ctx.files)
    // Every file the runtime stage installs: image path -> repository files, expanded from copied directories.
    const installed = new Map()
    for (const { dest, src } of runtimeContents(dockerfile, ctx.files)) {
      const inside = ctx.files.filter(f => f.startsWith(`${src}/`))
      for (const [path, file] of inside.length ? inside.map(f => [posix.join(dest, posix.relative(src, f)), f]) : [[dest, src]]) {
        installed.set(path, new Set([...(installed.get(path) ?? []), file]))
      }
    }
    const findings = []
    const seen = new Set()
    // A relative import resolves from where its importer is installed, so each installed copy is walked with its own location.
    const pending = [...installed].flatMap(([path, files]) => [...files].filter(f => ENTRY.test(f) && !NOT_SHIPPED.test(f)).map(file => ({ file, path })))
    while (pending.length) {
      const { file, path } = pending.pop()
      if (seen.has(path)) continue
      seen.add(path)
      const text = ctx.read(file) ?? ''
      for (const m of text.matchAll(IMPORT)) {
        const target = posix.normalize(posix.join(posix.dirname(file), m[1]))
        if (!tracked.has(target)) continue
        const resolved = posix.join(posix.dirname(path), m[1])
        if (installed.get(resolved)?.has(target)) pending.push({ file: target, path: resolved })
        else findings.push({ file, line: text.slice(0, m.index).split('\n').length, what: `imports ${target}, which the slice image does not copy at ${resolved} (next to its importer, installed at ${path})` })
      }
    }
    return findings
  },
}

// Every module a slice entrypoint imports must be copied into the slice image, or the slice host fails at start with a missing module.
import { posix } from 'node:path'

const DOCKERFILE = 'apps/kernel/slice-linux-docker/docker/Dockerfile'
const ENTRY = /^apps\/kernel\/slice-linux-docker\/docker\/[^/]+\.mjs$/
const NOT_SHIPPED = /\.(test|browser-test)\.mjs$/
const IMPORT = /(?:\bfrom\s*|\bimport\s*\(?\s*|\brequire\(\s*)['"](\.{1,2}\/[^'"]+)['"]/g

// Sources of COPY lines that come from the build context (not --from another stage).
function copiedSources(dockerfile) {
  const sources = []
  for (const line of dockerfile.replace(/\\\n/g, ' ').split('\n')) {
    const m = /^\s*COPY\s+(.*)$/.exec(line)
    if (!m) continue
    const words = m[1].trim().split(/\s+/)
    if (words.some(w => w.startsWith('--from'))) continue
    sources.push(...words.filter(w => !w.startsWith('--')).slice(0, -1).map(s => s.replace(/\/$/, '')))
  }
  return sources
}

export default {
  name: 'slice-image-closure',
  why: 'present before this check existed',
  rule: 'Review class: a module imported by the slice host but not shipped in the slice image',
  rootFix: `add a COPY line for the module to ${DOCKERFILE} next to its importer (or stop importing it from the slice host)`,
  scan(ctx) {
    const dockerfile = ctx.read(DOCKERFILE)
    if (dockerfile === null) return []
    const sources = copiedSources(dockerfile)
    const shipped = file => sources.some(s => file === s || file.startsWith(`${s}/`))
    const tracked = new Set(ctx.files)
    const findings = []
    const seen = new Set()
    const pending = ctx.files.filter(f => ENTRY.test(f) && !NOT_SHIPPED.test(f) && shipped(f))
    while (pending.length) {
      const file = pending.pop()
      if (seen.has(file)) continue
      seen.add(file)
      const text = ctx.read(file) ?? ''
      for (const m of text.matchAll(IMPORT)) {
        const target = posix.normalize(posix.join(posix.dirname(file), m[1]))
        if (!tracked.has(target)) continue
        if (!shipped(target)) findings.push({ file, line: text.slice(0, m.index).split('\n').length, what: `imports ${target}, which the slice image does not copy` })
        else pending.push(target)
      }
    }
    return findings
  },
}

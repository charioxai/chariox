// MP-08 / MP-09 / MP-10 / MP-11: refs are build inputs; workflow code stays on main.
export function stagingRef(value) {
  if (typeof value !== 'string') throw Error('main or staging union branch required')
  const ref = value.startsWith('refs/heads/') ? value : `refs/heads/${value}`
  if (ref === 'refs/heads/main') return ref
  if (!/^refs\/heads\/(?:stagenew|staging)\/[A-Za-z0-9][A-Za-z0-9._/-]*$/.test(ref) ||
      ref.includes('..') || ref.includes('//') || ref.split('/').some(part => part.startsWith('.') || part.endsWith('.lock')) || /[/.]$/.test(ref)) throw Error('main or staging union branch required')
  return ref
}

export function validateSources(source, components) {
  if (!source || Object.keys(source).sort().join(',') !== 'cloud,oss,relay') throw Error('release source identities required')
  for (const name of ['cloud', 'oss', 'relay']) {
    const item = source[name]
    if (!item || Object.keys(item).sort().join(',') !== 'commit,ref' || stagingRef(item.ref) !== item.ref ||
        !/^[0-9a-f]{40}$/.test(item.commit) || item.commit !== components[name].commit) throw Error('release source identity mismatch')
  }
  return source
}

export function requireMainRelease(manifest) {
  if (!manifest.source || ['cloud', 'oss', 'relay'].some(name => manifest.source[name]?.ref !== 'refs/heads/main')) throw Error('production requires a main release')
}

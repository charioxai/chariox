// MP-03/MP-07/MP-11: journal deployment-file access before changing live ownership.
import { constants } from 'node:fs'
import { chmod, chown, lstat, open, readFile, unlink } from 'node:fs/promises'
import { dirname, join } from 'node:path'

const names = ['bootstrap-receipt.json', 'bootstrap-grant-binding.json', 'release-override.json']
const [action, receipt, journal, legacyUid, legacyGid] = process.argv.slice(2)
const parent = dirname(receipt ?? '')
const snapshot = join(journal ?? '', 'previous-control-ownership.json')
function fail() { throw new Error('managed control ownership journal is unsafe') }
async function metadata(path, directory = false) {
  const s = await lstat(path).catch(e => { if (e.code === 'ENOENT' && !directory) return null; throw e })
  if (s === null) return null
  if (directory ? !s.isDirectory() : !s.isFile() || s.nlink !== 1) fail()
  if (s.mode & 0o022) fail()
  return { uid: s.uid, gid: s.gid, mode: s.mode & 0o7777 }
}
async function checkJournal() {
  const s = await lstat(journal)
  if (!s.isDirectory() || s.uid !== 0 || s.gid !== 0 || (s.mode & 0o077)) fail()
}
try {
  if (!receipt || !journal || !names.includes(receipt.slice(parent.length + 1))) fail()
  await checkJournal()
  if (action === 'snapshot') {
    const data = { version: 1, parent: await metadata(parent, true), files: {} }
    for (const name of names) data.files[name] = await metadata(join(parent, name))
    const file = await open(snapshot, constants.O_WRONLY | constants.O_CREAT | constants.O_EXCL | constants.O_NOFOLLOW, 0o600)
    try { await file.writeFile(JSON.stringify(data)); await file.sync() } finally { await file.close() }
  } else if (action === 'apply' || action === 'apply-legacy' || action === 'restore') {
    // Journals written before this migration have no ownership changes to undo.
    const s = await lstat(snapshot).catch(e => { if (e.code === 'ENOENT') return null; throw e })
    if (s === null && action === 'restore') process.exit(0)
    if (!s?.isFile() || s.uid !== 0 || s.gid !== 0 || s.nlink !== 1 || (s.mode & 0o077) || s.size > 4096) fail()
    const data = JSON.parse(await readFile(snapshot, 'utf8'))
    const valid = value => value && Object.keys(value).sort().join(',') === 'gid,mode,uid'
      && [value.uid, value.gid, value.mode].every(Number.isSafeInteger)
      && value.uid >= 0 && value.gid >= 0 && value.mode >= 0 && value.mode <= 0o7777 && !(value.mode & 0o022)
    if (data.version !== 1 || !valid(data.parent) || Object.keys(data.files ?? {}).sort().join(',') !== [...names].sort().join(',')) fail()
    for (const name of names) if (data.files[name] !== null && !valid(data.files[name])) fail()
    const legacy = { uid: Number(legacyUid), gid: Number(legacyGid), mode: 0o600 }
    if (action === 'apply-legacy' && (!valid(legacy) || legacyUid === undefined || legacyGid === undefined)) fail()
    // Validate every live entry before the first mutation; never follow links/aliases.
    await metadata(parent, true)
    const live = new Map()
    for (const name of names) live.set(name, await metadata(join(parent, name)))
    for (const name of names) {
      const path = join(parent, name)
      if (!live.get(name)) continue
      const original = data.files[name]
      if (action === 'restore' && original === null) { await unlink(path); continue }
      const target = action === 'apply' ? { uid: 0, gid: 0, mode: 0o600 } : action === 'apply-legacy' ? legacy : original
      await chown(path, target.uid, target.gid)
      await chmod(path, target.mode)
      const file = await open(path, constants.O_RDONLY | constants.O_NOFOLLOW | constants.O_NONBLOCK)
      try { await file.sync() } finally { await file.close() }
    }
    const target = action === 'apply' ? { uid: 0, gid: 0, mode: 0o700 } : action === 'apply-legacy' ? {...legacy, mode: 0o700} : data.parent
    await chown(parent, target.uid, target.gid)
    await chmod(parent, target.mode)
    const directory = await open(parent, constants.O_RDONLY | constants.O_DIRECTORY | constants.O_NOFOLLOW)
    try { await directory.sync() } finally { await directory.close() }
  } else fail()
} catch {
  console.error('MP-03/MP-07 managed control ownership migration failed')
  process.exitCode = 1
}

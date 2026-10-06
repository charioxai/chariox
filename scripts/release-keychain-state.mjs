import fs from 'node:fs'
import path from 'node:path'
import { execFileSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'

function security(args) {
  return execFileSync('security', args, { encoding: 'utf8', stdio: ['ignore', 'pipe', 'ignore'] })
}
function paths(output) {
  const values = [...output.matchAll(/"([^"\r\n]+)"/g)].map(match => match[1])
  if (!values.length || values.some(value => !path.isAbsolute(value))) throw new Error('keychain inventory is invalid')
  return values
}
export function snapshotKeychains(root) {
  const keychain = path.join(root, 'release.keychain-db')
  if (fs.existsSync(keychain)) throw new Error('release keychain path is already occupied')
  const state = { keychain, search: paths(security(['list-keychains', '-d', 'user'])), default: paths(security(['default-keychain', '-d', 'user']))[0] }
  fs.writeFileSync(path.join(root, 'release-keychain-state.json'), JSON.stringify(state), { flag: 'wx', mode: 0o600 })
}
export function cleanupKeychains(root) {
  const statePath = path.join(root, 'release-keychain-state.json')
  if (!fs.existsSync(statePath)) return
  const state = JSON.parse(fs.readFileSync(statePath, 'utf8'))
  if (state.keychain !== path.join(root, 'release.keychain-db')) throw new Error('release keychain ownership mismatch')
  const failures = []
  for (const args of [['list-keychains', '-d', 'user', '-s', ...state.search], ['default-keychain', '-d', 'user', '-s', state.default], ['delete-keychain', state.keychain]]) {
    try { security(args) } catch { failures.push('keychain cleanup failed') }
  }
  fs.rmSync(path.join(root, 'identity.p12'), { force: true })
  fs.rmSync(path.join(root, 'notary.p8'), { force: true })
  if (failures.length) throw new Error('keychain cleanup requires recovery')
  fs.rmSync(statePath)
}
if (process.argv[1] && fileURLToPath(import.meta.url) === path.resolve(process.argv[1])) {
  try {
    const [, , action, root] = process.argv
    if (!root || !path.isAbsolute(root)) throw new Error('release scratch root must be absolute')
    if (action === 'snapshot') snapshotKeychains(root)
    else if (action === 'cleanup') cleanupKeychains(root)
    else throw new Error('unknown keychain state action')
  } catch { process.stderr.write('release keychain state operation failed\n'); process.exitCode = 1 }
}

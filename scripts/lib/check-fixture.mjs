// Throwaway git repositories for the rule-check tests: a base commit with the given files, then the change under test.
import { execFileSync } from 'node:child_process'
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const runner = fileURLToPath(new URL('../checks/run.mjs', import.meta.url))
const git = (root, ...args) => execFileSync('git', ['-c', 'user.name=t', '-c', 'user.email=t@example.test', ...args], { cwd: root, encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] }).trim()

export function makeRepo(files) {
  const root = mkdtempSync(join(tmpdir(), 'rule-check-'))
  git(root, 'init', '-q', '-b', 'main')
  const repo = {
    root,
    write(path, content) {
      mkdirSync(dirname(join(root, path)), { recursive: true })
      writeFileSync(join(root, path), content)
      git(root, 'add', '-A')
    },
    commit: message => { git(root, 'commit', '-q', '-m', message); return git(root, 'rev-parse', 'HEAD') },
    // Runs one check against the working tree, comparing with `base`. Returns {status, text}.
    run(name, { base = repo.base, args = [] } = {}) {
      try {
        const out = execFileSync('node', [runner, '--root', root, '--only', name, ...args], { encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'], env: { ...process.env, CHARIOX_BOUNDARY_BASE: base, CI: '' } })
        return { status: 0, text: out }
      } catch (error) { return { status: error.status, text: `${error.stdout}${error.stderr}` } }
    },
    dispose: () => rmSync(root, { recursive: true, force: true }),
  }
  for (const [path, content] of Object.entries(files)) repo.write(path, content)
  repo.base = repo.commit('base')
  return repo
}

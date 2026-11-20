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
    run: (name, { base = repo.base, args = [] } = {}) => runCheck(root, name, base, args),
    // A clone of `ref` the way CI checks a pull request out: that one commit, plus `base` fetched at depth 1. Its `run`
    // compares with `base`; its origin is this repository.
    shallowClone(ref, base = repo.base) {
      git(root, 'config', 'uploadpack.allowReachableSHA1InWant', 'true')
      const clone = mkdtempSync(join(tmpdir(), 'rule-check-clone-'))
      git(clone, 'clone', '-q', '--depth=1', '--no-tags', '--branch', ref, `file://${root}`, '.')
      git(clone, 'fetch', '-q', '--depth=1', '--no-tags', 'origin', base)
      return {
        root: clone,
        git: (...args) => git(clone, ...args),
        run: (name, { args = [] } = {}) => runCheck(clone, name, base, args),
        dispose: () => rmSync(clone, { recursive: true, force: true }),
      }
    },
    git: (...args) => git(root, ...args),
    dispose: () => rmSync(root, { recursive: true, force: true }),
  }
  for (const [path, content] of Object.entries(files)) repo.write(path, content)
  repo.base = repo.commit('base')
  return repo
}

function runCheck(root, name, base, args) {
  try {
    const out = execFileSync('node', [runner, '--root', root, '--only', name, ...args], { encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'], env: { ...process.env, CHARIOX_BOUNDARY_BASE: base, CI: '' } })
    return { status: 0, text: out }
  } catch (error) { return { status: error.status, text: `${error.stdout}${error.stderr}` } }
}

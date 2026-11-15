// Real-provider room command coverage for SPAWNWT-01/02/03, used by orchestrate-one-room.mjs.
import assert from 'node:assert/strict'
import { execFileSync } from 'node:child_process'
import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { submitPromptRequest } from '../../packages/kernel-client/dist/ipc-requests.js'
import { listDescendantsRequest } from '../../packages/kernel-client/dist/ipc-agent-requests.js'

export async function runSiblingWorktrees({ a, R, orch, attached, provider, profile, model, work, note, until, evidence }) {
  const git = (directory, ...args) => execFileSync('git', [
    '-C', directory, '-c', 'user.name=drill', '-c', 'user.email=drill@example.invalid', ...args,
  ], { stdio: ['ignore', 'pipe', 'pipe'] }).toString().trim()
  const sibling = join(dirname(work), 'existing-worker')
  const created = join(dirname(work), 'created-worker')
  const unrelated = join(dirname(work), 'unrelated')
  const otherRepo = join(dirname(work), 'other-repo')
  git(work, 'worktree', 'add', '-b', 'existing-worker', sibling)
  mkdirSync(unrelated)
  mkdirSync(otherRepo)
  git(otherRepo, 'init', '-q')
  git(otherRepo, 'commit', '-q', '--allow-empty', '-m', 'unrelated')
  const report = join(work, 'spawnwt-report.json')
  const commonFlags = `--provider ${provider} --account-profile ${profile} --model ${model} --effort low`
  const brief = `Use the Chariox room run_command tool for the following four commands, once each. Do not retry refusals or use shell commands to spawn agents. Do not ask the owner anything.
1. agent spawn existing ${commonFlags} --dir ${sibling}
2. agent spawn created ${commonFlags} --worktree ${created} --branch created-worker
3. agent spawn unrelated ${commonFlags} --dir ${unrelated}
4. agent spawn other ${commonFlags} --dir ${otherRepo}
Write ${report} as JSON with keys existing, created, unrelated, other, each containing the exact corresponding tool answer. Include refusals. Then for each SUCCESSFULLY spawned worker (existing and created only), use the room command prompt <alias> with this task: "Use your shell to run pwd > spawnwt-cwd.txt in your current directory, then end with DONE. Do not spawn anything or ask questions."
End this turn. If woken by a worker result, read and acknowledge the inbox then end the turn. Do not delete workers, subscribe or wait.`
  const briefPath = join(work, 'BRIEF.md')
  writeFileSync(briefPath, brief)
  await a.rpc(submitPromptRequest(R, attached, orch, `Read ${briefPath} and carry it out now.`, []))
  await until(() => existsSync(report), 'sibling worktree command report', 240000)
  const answers = JSON.parse(readFileSync(report, 'utf8'))
  writeFileSync(join(evidence, 'spawnwt-answers.json'), JSON.stringify(answers, null, 2) + '\n')
  const tree = (await a.rpc(listDescendantsRequest(R, orch))).AgentTree.tree
  writeFileSync(join(evidence, 'spawnwt-tree.json'), JSON.stringify(tree, null, 2) + '\n')
  const nodes = tree.nodes
  const check = (cell, ok, detail = {}) => {
    note(cell, { ok, ...detail })
    assert(ok, `${cell}: ${JSON.stringify(detail)}`)
  }
  check('SPAWNWT-01', nodes.some(n => n.alias === 'existing' && n.worktree_id === sibling))
  check('SPAWNWT-02', nodes.some(n => n.alias === 'created' && n.worktree_id === created)
    && git(created, 'branch', '--show-current') === 'created-worker'
    && git(created, 'rev-parse', '--path-format=absolute', '--git-common-dir')
      === git(work, 'rev-parse', '--path-format=absolute', '--git-common-dir'))
  check('SPAWNWT-03', !nodes.some(n => ['unrelated', 'other'].includes(n.alias))
    && ['unrelated', 'other'].every(key => JSON.stringify(answers[key]).includes('same Git repository')))
  await until(() => [sibling, created].every(p => existsSync(join(p, 'spawnwt-cwd.txt'))), 'workers run in their worktrees', 240000)
  for (const directory of [sibling, created]) {
    check('worker-cwd', readFileSync(join(directory, 'spawnwt-cwd.txt'), 'utf8').trim() === directory, { directory })
  }
}

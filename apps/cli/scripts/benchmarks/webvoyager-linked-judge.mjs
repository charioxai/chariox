// MP-08/MP-10/MP-11: unchanged official prompts, approved substitute CLI judge.
import assert from 'node:assert/strict'
import { spawn, execFile } from 'node:child_process'
import { readFile, writeFile } from 'node:fs/promises'
import { promisify } from 'node:util'
import path from 'node:path'
import { stopOwnedProcess } from './round2/owned-processes.mjs'
import { nativeFailureEvidence } from './round2/provider-availability.mjs'
const exec = promisify(execFile)

export async function judgeWebVoyager({ task, directory, screenshots, upstream, root, accountHome, observationPolicy }) {
  const images = screenshots.slice(-15)
  await exec('python3', [path.join(import.meta.dirname, 'webvoyager-judge.py'), `${upstream}/evaluation/auto_eval.py`,
    task.ques, `${directory}/answer.txt`, String(images.length), `${directory}/judge-input.json`])
  const input = JSON.parse(await readFile(`${directory}/judge-input.json`, 'utf8'))
  const args = ['exec', '--ignore-user-config', '--ignore-rules', '--ephemeral', '--skip-git-repo-check',
    '--sandbox', 'read-only', '--model', 'gpt-6.1-sol', '-c', 'model_reasoning_effort="low"',
    '-c', `developer_instructions=${JSON.stringify(input.system)}`, '--json', '-o', `${directory}/judge-answer.txt`]
  for (const image of images) args.push('--image', `${directory}/${image}`)
  args.push('-')
  const start = Date.now()
  const child = spawn('codex', args, { cwd: root, detached: true, env: { PATH: process.env.PATH,
    HOME: `${root}/home`, CODEX_HOME: accountHome, LANG: 'C.UTF-8' }, stdio: ['pipe', 'pipe', 'ignore'] })
  await new Promise((resolve, reject) => { child.once('spawn', resolve); child.once('error', reject) })
  assert(Number.isSafeInteger(child.pid) && child.pid > 1)
  let output = '', abort, exitCode, firstError
  const exited = new Promise(resolve => child.once('exit', code => resolve(code)))
  const timer = setTimeout(() => { abort = 'deadline'; void stopOwnedProcess(child, { detached: true }).catch(() => {}) }, 120000)
  child.stdout.on('data', data => {
    output += data
    if (Buffer.byteLength(output) > 4 * 1024 ** 2 && !abort) {
      abort = 'output_limit'; void stopOwnedProcess(child, { detached: true }).catch(() => {})
    }
  })
  child.stdin.on('error', () => { abort ??= 'stdin' })
  child.stdin.end(input.user)
  try {
    exitCode = await exited
    assert(!abort && exitCode === 0, 'MP-10 substitute judge failed')
    const events = output.trim().split('\n').map(line => JSON.parse(line))
    const tools = events.filter(event => event.type === 'item.completed' && !['agent_message', 'reasoning'].includes(event.item?.type)).length
    const answer = await readFile(`${directory}/judge-answer.txt`, 'utf8')
    const verdict = !tools && /SUCCESS/.test(answer) ? answer.includes('NOT SUCCESS') ? 'NOT SUCCESS' : 'SUCCESS' : null
    const result = { ...(observationPolicy ? { observationPolicy } : {}), mpItems: ['MP-08', 'MP-10', 'MP-11'], judgeModel: 'gpt-6.1-sol', judgeEffort: 'low',
      judgeToolCalls: tools, judgeUsage: events.find(event => event.type === 'turn.completed')?.usage ?? null,
      judgeWallSeconds: (Date.now() - start) / 1000, judgeAnswer: answer, judgeVerdict: verdict, judgeValid: verdict !== null }
    await writeFile(`${directory}/judge-audit.json`, JSON.stringify(result, null, 2), { mode: 0o600 })
    return result
  } catch (error) {
    firstError = error
    error.judgeFailure = { ...nativeFailureEvidence({ output, exitCode, abort }), ...(observationPolicy ? { observationPolicy } : {}) }
    try { await writeFile(`${directory}/judge-failure.json`, JSON.stringify(error.judgeFailure, null, 2) + '\n', { mode: 0o600 }) }
    catch { error.judgeFailure.evidenceWriteFailed = true }
    throw error
  } finally {
    clearTimeout(timer)
    try { assert(await stopOwnedProcess(child, { detached: true }), 'MP-11 judge exit acknowledgement missing') }
    catch (error) {
      const original = firstError ?? error
      original.judgeFailure ??= nativeFailureEvidence({ output, exitCode, abort })
      Object.assign(original.judgeFailure, { cleanupFailed: true, ownedPid: child.pid, cleanupFailureClass: error.name })
      try { await writeFile(`${directory}/judge-failure.json`, JSON.stringify(original.judgeFailure, null, 2) + '\n', { mode: 0o600 }) }
      catch { original.judgeFailure.evidenceWriteFailed = true }
      throw original
    }
  }
}

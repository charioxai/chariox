// MP-08/MP-10/MP-11: exercise the actual native judge failure before evidence is lost.
import assert from 'node:assert/strict'
import { test } from 'node:test'
import { mkdtemp, mkdir, writeFile, readFile, rm } from 'node:fs/promises'
import os from 'node:os'
import path from 'node:path'
import { judgeWebVoyager } from './webvoyager-linked-judge.mjs'

test('MP-08/MP-10/MP-11 nonzero judge exit retains original error and quota classification', async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), 'r2next-judge-failure-'))
  const previousPath = process.env.PATH
  try {
    const bin = `${root}/bin`, directory = `${root}/attempt`, upstream = `${root}/upstream`
    await Promise.all([bin, directory, `${upstream}/evaluation`].map(p => mkdir(p, { recursive: true })))
    await writeFile(`${directory}/answer.txt`, 'fixture answer')
    await writeFile(`${upstream}/evaluation/auto_eval.py`, 'SYSTEM_PROMPT = "fixture official system"\nUSER_PROMPT = "<task> <answer> <num>"\n')
    const message = 'Provider usage is exhausted (weekly). Bearer syntheticfixturesecret123456'
    await writeFile(`${bin}/codex`, `#!/usr/bin/env node\nprocess.stdout.write(${JSON.stringify(JSON.stringify({ type: 'turn.failed', error: { message } }) + '\n')});process.exitCode=1\n`, { mode: 0o700 })
    process.env.PATH = `${bin}:${previousPath}`
    await assert.rejects(judgeWebVoyager({ task: { ques: 'fixture task' }, directory, screenshots: [], upstream, root, accountHome: `${root}/profile` }))
    const failure = JSON.parse(await readFile(`${directory}/judge-failure.json`, 'utf8'))
    assert.equal(failure.exitCode, 1)
    assert.equal(failure.available, false)
    assert.equal(failure.usageExhausted, true)
    assert.equal(failure.errorEvents[0].type, 'turn.failed')
    assert.match(failure.errorEvents[0].error.message, /weekly/)
    assert.match(failure.errorEvents[0].error.message, /<redacted>/)
    assert(!JSON.stringify(failure).includes('syntheticfixturesecret123456'))
  } finally {
    process.env.PATH = previousPath
    await rm(root, { recursive: true })
  }
})

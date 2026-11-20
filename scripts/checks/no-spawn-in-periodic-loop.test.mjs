import assert from 'node:assert/strict'
import { test } from 'node:test'
import { makeRepo } from '../lib/check-fixture.mjs'
import check from './no-spawn-in-periodic-loop.mjs'

const FILE = 'apps/kernel/src/runtime/sampler.rs'
const red = (text, expected) => {
  const r = makeRepo({ [FILE]: 'fn sample() {\n    loop {\n        read_proc();\n        std::thread::sleep(INTERVAL);\n    }\n}\n' })
  assert.equal(r.run(check.name).status, 0, 'the clean tree passes')
  r.write(FILE, text)
  const result = r.run(check.name)
  assert.equal(result.status, 1)
  assert.match(result.text, expected)
  assert.ok(result.text.includes(`Root fix: ${check.rootFix}`))
  r.dispose()
}

test('a periodic loop that starts a child is red', () => {
  red('fn sample() {\n    loop {\n        let out = crate::process_spawn::command("ps").output();\n        std::thread::sleep(INTERVAL);\n    }\n}\n', /sampler\.rs:3: periodic loop starts a subprocess/)
  red('async fn sample() {\n    let mut ticks = tokio::time::interval(INTERVAL);\n    while running() {\n        ticks.tick().await;\n        Command::new("ps").status().await;\n    }\n}\n', /sampler\.rs:5: periodic loop starts a subprocess/)
})

test('a periodic loop that calls a function of the same file that starts a child is red', () => {
  red('fn sample() {\n    loop {\n        let rows = rows();\n        std::thread::sleep(INTERVAL);\n    }\n}\nfn rows() -> Vec<Row> { parse(run_ps()) }\nfn run_ps() -> Output { crate::process_spawn::command("ps").output() }\n', /sampler\.rs:3: periodic loop calls rows, which starts a subprocess/)
})

test('a child started once, a loop without a clock, and test code pass', () => {
  const r = makeRepo({
    [FILE]: 'fn start() {\n    let child = Command::new("tail").spawn();\n    loop {\n        std::thread::sleep(INTERVAL);\n    }\n}\nfn drain(rx: Receiver) {\n    while let Ok(job) = rx.recv() {\n        Command::new("cp").status();\n    }\n}\n#[cfg(test)]\nmod tests {\n    fn t() { loop { Command::new("ps"); sleep(D); } }\n}\n',
  })
  assert.equal(r.run(check.name).status, 0)
  r.dispose()
})

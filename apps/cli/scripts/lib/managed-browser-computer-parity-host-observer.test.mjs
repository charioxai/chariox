import assert from "node:assert/strict"
import test from "node:test"
import { runBoundedObserverCommand, observeManagedParityHost } from "./managed-browser-computer-parity-host-observer.mjs"

test("host observer rejects option injection before executing SSH", async () => {
  for (const host of ["-oProxyCommand=bad", "host; bad", "host\ncommand"]) {
    await assert.rejects(observeManagedParityHost({ host }), /explicit SSH host/)
  }
})

test("host observer refuses ambient Docker configuration", async () => {
  for (const engine of [null, { endpoint: "unix:///run/chariox-docker/docker.sock" },
    { endpoint: "tcp://elsewhere:2375", id: "engine-123" }]) {
    await assert.rejects(observeManagedParityHost({ host: "trusted-host", engine }), /pinned Unix Docker/)
  }
})

test("bounded observer command passes input and returns only stdout", async () => {
  const output = await runBoundedObserverCommand(process.execPath, ["-e", `
    process.stdin.pipe(process.stdout); process.stderr.write('private diagnostic');
  `], { input: '{"owned":"receipt"}' })
  assert.equal(output, '{"owned":"receipt"}')
})

test("bounded observer kills a stalled process group and waits for close", async () => {
  const started = Date.now()
  await assert.rejects(runBoundedObserverCommand(process.execPath, ["-e", `
    process.on('SIGTERM', () => {});
    require('node:child_process').spawn(process.execPath, ['-e',
      "process.on('SIGTERM', () => {}); setInterval(() => {}, 1000)"], {stdio: 'inherit'});
    setInterval(() => {}, 1000);
  `], { timeoutMs: 100 }), /deadline exceeded/)
  assert.ok(Date.now() - started < 2000)
})

test("observer abort and excessive output fail without exposing subprocess diagnostics", async () => {
  const controller = new AbortController()
  const pending = runBoundedObserverCommand(process.execPath, ["-e", "setInterval(() => {}, 1000)"], {
    signal: controller.signal,
  })
  controller.abort()
  await assert.rejects(pending, /aborted/)
  await assert.rejects(runBoundedObserverCommand(process.execPath, ["-e", `
    process.stdout.write('x'.repeat(3 * 1024 * 1024)); setInterval(() => {}, 1000);
  `]), /output limit exceeded/)
})

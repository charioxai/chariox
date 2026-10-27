import assert from "node:assert/strict"
import test from "node:test"
import { fileURLToPath } from "node:url"
import { runBoundedObserverCommand, observeManagedParityHost } from "./managed-browser-computer-parity-host-observer.mjs"

test("host observer rejects option injection before executing SSH", async () => {
  for (const host of ["-oProxyCommand=bad", "host; bad", "host\ncommand"]) {
    await assert.rejects(observeManagedParityHost({ host }), /explicit SSH host/)
  }
})

test("physical census reports missing root access before inspecting processes", async () => {
  await assert.rejects(runBoundedObserverCommand("python3", ["-B", "-c",
    "import os,runpy,sys; os.geteuid=lambda:1001; runpy.run_path(sys.argv[1],run_name='__main__')",
    fileURLToPath(new URL("./managed-browser-computer-parity-host-probe.py", import.meta.url))]), /requires a root SSH identity/)
})

test("host observer refuses ambient Docker configuration", async () => {
  for (const engine of [null, { endpoint: "unix:///run/chariox-docker/docker.sock" },
    { endpoint: "tcp://elsewhere:2375", id: "engine-123" }]) {
    await assert.rejects(observeManagedParityHost({ host: "trusted-host", engine }), /pinned Unix Docker/)
  }
})

test("physical cgroup ownership rejects host scope and container-ID substrings", async () => {
  await runBoundedObserverCommand("python3", ["-B", "-c", `
import importlib.util, sys
spec = importlib.util.spec_from_file_location("probe", sys.argv[1])
probe = importlib.util.module_from_spec(spec)
spec.loader.exec_module(probe)
identity = "a" * 64
assert probe.belongs_to_container("0::/user.slice/docker-" + identity + ".scope", {identity})
assert not probe.belongs_to_container("0::/user.slice", {identity})
assert not probe.belongs_to_container("0::/docker-" + identity + "a.scope", {identity})
assert not probe.belongs_to_container("0::/docker-a" + identity + ".scope", {identity})
before = {"pid": 42, "startTicks": "100", "pidNamespace": "pid:[1]", "netNamespace": "net:[1]"}
escaped = {**before, "pidNamespace": "pid:[2]", "netNamespace": "net:[2]"}
assert not probe.same_process(before, escaped)
assert probe.same_process_lifetime(before, escaped)
assert not probe.same_process_lifetime(before, {**escaped, "startTicks": "101"})
`, fileURLToPath(new URL("./managed-browser-computer-parity-host-probe.py", import.meta.url))])
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

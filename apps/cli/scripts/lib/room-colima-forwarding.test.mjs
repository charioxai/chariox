import assert from "node:assert/strict"
import { spawn } from "node:child_process"
import { once } from "node:events"
import net from "node:net"
import test from "node:test"
import { startRoomColimaForwarding, startRoomSliceWithForwarding } from "./room-colima-forwarding.mjs"

async function freePort() {
  const server = net.createServer()
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve))
  const port = server.address().port
  await new Promise((resolve) => server.close(resolve))
  return port
}

async function available(port) {
  const server = net.createServer()
  return await new Promise((resolve) => {
    server.once("error", () => resolve(false))
    server.listen(port, "127.0.0.1", () => server.close(() => resolve(true)))
  })
}

function slice(port) {
  return { backend: "local_docker", name: "room-pointer-123-test", owner_kernel_id: "room-pointer-123-test-home",
    local_docker_ports: { relay: port, kernel: port + 1, novnc: port + 2 } }
}

test("forwarding is opt-in and owns listeners until idempotent cleanup", async () => {
  assert.equal(await startRoomColimaForwarding({}), null)
  const port = await freePort()
  let requested
  const forward = await startRoomColimaForwarding({ sshConfig: "/fixture/ssh-config", slice: slice(port) }, {
    spawn(program, args, options) {
      requested = { program, args }
      // Exercise a real subprocess/listener lifecycle at the SSH boundary.
      return spawn(process.execPath, ["-e", `
        const net=require('node:net');
        let ready=0;
        for(const port of ${JSON.stringify([port, port + 1, port + 2])}) {
          const server=net.createServer(socket=>socket.end());
          server.listen(port,'127.0.0.1',()=>{
            process.stderr.write('debug1: Local forwarding listening on 127.0.0.1 port '+port+'.\\r\\n');
            if(++ready===3)process.stderr.write('debug1: Entering interactive session.\\r\\n');
          });
        }
      `], options)
    },
  })
  try {
    forward.assertHealthy()
    assert.equal(requested.program, "ssh")
    assert.ok(requested.args.includes("ControlPath=none"))
    assert.ok(requested.args.includes("ControlMaster=no"))
    assert.ok(requested.args.includes("ControlPersist=no"))
    assert.deepEqual(forward.ports, [port, port + 1, port + 2])
    assert.equal(await available(port), false)
  } finally { await forward.close() }
  await forward.close()
  for (const owned of forward.ports) assert.equal(await available(owned), true)
})

test("invalid scope and ports never start a process", async () => {
  for (const input of [
    { sshConfig: "relative", slice: slice(45000) },
    { sshConfig: "/fixture", slice: { ...slice(45000), owner_kernel_id: "other" } },
    { sshConfig: "/fixture", slice: { ...slice(45000), backend: "remote" } },
    { sshConfig: "/fixture", slice: slice(65535) },
    { sshConfig: "/fixture", slice: { ...slice(45000), local_docker_ports: { ...slice(45000).local_docker_ports, codex_range_start: 65520 } } },
  ]) {
    await assert.rejects(startRoomColimaForwarding(input, { spawn() { assert.fail("must not start") } }))
  }
})

test("partial startup failure closes its listener and never exposes SSH diagnostics", async () => {
  const port = await freePort()
  await assert.rejects(startRoomColimaForwarding({ sshConfig: "/fixture", slice: slice(port) }, {
    spawn(_, __, options) {
      return spawn(process.execPath, ["-e", `
        require('node:net').createServer().listen(${port},'127.0.0.1',()=>{
          process.stderr.write('debug1: Local forwarding listening on 127.0.0.1 port ${port}.\\nPRIVATE-CONFIG-DATA\\n');
          setTimeout(()=>process.exit(1),20);
        });
      `], options)
    },
  }), (error) => /exited during startup/.test(error.message) && !error.message.includes("PRIVATE"))
  assert.equal(await available(port), true)
})

test("unexpected forward exit cannot be reported as healthy", async () => {
  const port = await freePort()
  let child
  const forward = await startRoomColimaForwarding({ sshConfig: "/fixture", slice: slice(port) }, {
    spawn(_, __, options) {
      child = spawn(process.execPath, ["-e", `
        const ports=${JSON.stringify([port, port + 1, port + 2])};
        for(const p of ports)process.stderr.write('debug1: Local forwarding listening on 127.0.0.1 port '+p+'.\\n');
        process.stderr.write('debug1: Entering interactive session.\\n');setInterval(()=>{},1000);
      `], options)
      return child
    },
  })
  const exited = once(child, "exit")
  child.kill("SIGTERM")
  await exited
  assert.throws(() => forward.assertHealthy(), /not active/)
  await forward.close()
})

test("a silent startup times out and cleans up the real child", { timeout: 15000 }, async () => {
  const port = await freePort()
  let child
  await assert.rejects(startRoomColimaForwarding({ sshConfig: "/fixture", slice: slice(port) }, {
    spawn(_, __, options) {
      child = spawn(process.execPath, ["-e", "setInterval(()=>{},1000)"], options)
      return child
    },
  }), /startup timed out/)
  assert.ok(child.exitCode !== null || child.signalCode !== null)
})

test("cleanup kills only its own child if graceful termination is ignored", async () => {
  const port = await freePort()
  let child
  const forward = await startRoomColimaForwarding({ sshConfig: "/fixture", slice: slice(port) }, {
    spawn(_, __, options) {
      child = spawn(process.execPath, ["-e", `
        process.on('SIGTERM',()=>{});
        const ports=${JSON.stringify([port, port + 1, port + 2])};let ready=0;
        for(const p of ports)require('node:net').createServer().listen(p,'127.0.0.1',()=>{
          process.stderr.write('debug1: Local forwarding listening on 127.0.0.1 port '+p+'.\\n');
          if(++ready===3)process.stderr.write('debug1: Entering interactive session.\\n');
        });
      `], options)
      return child
    },
  })
  await forward.close()
  assert.equal(child.signalCode, "SIGKILL")
  for (const owned of forward.ports) assert.equal(await available(owned), true)
})

test("forwarding waits for creation and a failed setup waits for provisioning to settle", async () => {
  const port = await freePort()
  let created = false
  let settled = false
  let finishStart
  await assert.rejects(startRoomSliceWithForwarding({
    sshConfig: "/fixture", slice: slice(port),
    async startSlice() {
      assert.equal(await available(port), true, "kernel preflight must see a free port")
      created = true
      await new Promise((resolve) => { finishStart = resolve })
      settled = true
    },
    async containerExists() { return created },
  }, {
    spawn(_, __, options) {
      assert.equal(created, true)
      assert.equal(settled, false)
      const child = spawn(process.execPath, ["-e", "process.exit(1)"], options)
      child.once("exit", () => setTimeout(finishStart, 10))
      return child
    },
  }), /exited during startup/)
  assert.equal(settled, true, "caller cleanup cannot race the provisioner")
  assert.equal(await available(port), true)
})

test("a kernel failure before creation cannot start a late forward", async () => {
  const port = await freePort()
  await assert.rejects(startRoomSliceWithForwarding({
    sshConfig: "/fixture", slice: slice(port),
    async startSlice() { throw new Error("kernel-start-failed") },
    async containerExists() { return false },
  }, { spawn() { assert.fail("no late SSH process") } }), /kernel-start-failed/)
})

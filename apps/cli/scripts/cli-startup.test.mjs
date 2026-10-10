// MP-08 / MP-10 / MP-11: admission runs before importing the TUI/runtime.
import assert from "node:assert/strict"
import { spawn } from "node:child_process"
import { mkdtemp, mkdir, readFile, readdir, rm, writeFile } from "node:fs/promises"
import net from "node:net"
import os from "node:os"
import path from "node:path"
import { fileURLToPath } from "node:url"
import { once } from "node:events"
import { setTimeout as sleep } from "node:timers/promises"
import test from "node:test"
import { createRequire } from "node:module"
const require = createRequire(import.meta.url)
const dependencyRoots = [path.dirname(fileURLToPath(import.meta.url)), process.env.CHARIOX_TEST_NODE_MODULES].filter(Boolean)
const { transformAsync } = require(require.resolve("@babel/core", { paths: dependencyRoots }))
const tsPreset = require(require.resolve("@babel/preset-typescript", { paths: dependencyRoots }))

const source = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../src")
async function bind(port = 0) {
  const server = net.createServer()
  try {
    server.listen(port, "127.0.0.1")
    await once(server, "listening")
    return server
  } catch (error) { server.close(); throw error }
}
const close = (server) => new Promise(resolve => server.close(resolve))

for (const args of [["--help"], ["-h"], ["--unknown"], ["unexpected"], ["--workspace", "/tmp", "--unknown"], ["--help", "--unknown"], ["--workspace"]]) {
  test(`CLI entry rejects runtime admission for ${JSON.stringify(args)}`, async () => {
    const root = await mkdtemp(path.join(os.tmpdir(), "chariox-cli-startup-"))
    try {
      const code = path.join(root, "code")
      await mkdir(code)
      await writeFile(path.join(code, "package.json"), '{"type":"module"}')
      for (const name of ["index", "cli-options", "app-command-catalog"]) {
        const result = await transformAsync(await readFile(path.join(source, `${name}.ts`), "utf8"), { filename: `${name}.ts`, presets: [tsPreset] })
        await writeFile(path.join(code, `${name}.js`), result.code)
      }
      // Only the runtime boundary is replaced: any import past admission writes
      // state and binds a real socket, so a help/error path cannot pass silently.
      await writeFile(path.join(code, "cli-main.js"), `import fs from "node:fs"; import net from "node:net"; fs.mkdirSync(process.env.HOME,{recursive:true}); fs.writeFileSync(process.env.HOME+"/admitted","runtime"); net.createServer().listen(Number(process.env.CHARIOX_KERNEL_PORT),"127.0.0.1");`)
      await writeFile(path.join(code, "runner.mjs"), 'globalThis.Bun={plugin(){}}; await import("./index.js");')
      for (const occupied of [false, true]) {
        const empty = path.join(root, occupied ? "occupied" : "free")
        await mkdir(empty)
        const reservation = await bind()
        const port = reservation.address().port
        if (!occupied) await close(reservation)
        const child = spawn(process.execPath, [path.join(code, "runner.mjs"), ...args], {
          env: { HOME: path.join(empty, "home"), CHARIOX_HOME: path.join(empty, "chariox"), CHARIOX_LOG_DIR: path.join(empty, "logs"), CHARIOX_KERNEL_PORT: String(port) },
          stdio: ["ignore", "pipe", "pipe"],
        })
        let stdout = "", stderr = "", ended = false, bound = false, timedOut = false
        child.stdout.on("data", chunk => { stdout += chunk })
        child.stderr.on("data", chunk => { stderr += chunk })
        const settled = once(child, "close").then(([exit]) => { ended = true; return exit })
        const deadline = Date.now() + 2000
        while (!ended && Date.now() < deadline) {
          if (!occupied) {
            try { await close(await bind(port)) } catch (error) {
              if (error.code !== "EADDRINUSE") throw error
              bound = true
            }
          }
          await sleep(5)
        }
        if (!ended) {
          timedOut = true
          assert.ok(Number.isSafeInteger(child.pid) && child.pid > 1, "refuse reserved PID")
          child.kill("SIGKILL")
        }
        const exit = await settled
        if (occupied) await close(reservation)
        assert.equal(bound, false, "CLI bound a socket")
        assert.deepEqual(await readdir(empty), [], "CLI created runtime files")
        assert.equal(timedOut, false, "CLI started instead of exiting")
        const help = args.length === 1 && ["--help", "-h"].includes(args[0])
        assert.equal(exit === 0, help, stderr)
        assert.match(help ? stdout : stderr, help ? /usage: chariox/ : /unknown argument|missing value/)
        if (help) assert.equal(stderr, "")
      }
    } finally { await rm(root, { recursive: true, force: true }) }
  })
}

// Kernel subcommands own their parsers; the TUI option gate must not reject them.
for (const args of [["access", "list"], ["sudo", "request"]]) {
  test(`CLI entry admits ${args[0]} to its subcommand`, async () => {
    const root = await mkdtemp(path.join(os.tmpdir(), "chariox-cli-startup-"))
    try {
      const code = path.join(root, "code")
      await mkdir(code)
      await writeFile(path.join(code, "package.json"), '{"type":"module"}')
      for (const name of ["index", "cli-options", "app-command-catalog"]) {
        const result = await transformAsync(await readFile(path.join(source, `${name}.ts`), "utf8"), { filename: `${name}.ts`, presets: [tsPreset] })
        await writeFile(path.join(code, `${name}.js`), result.code)
      }
      await writeFile(path.join(code, "cli-main.js"), `process.stdout.write("admitted:" + process.argv.slice(2).join(" "))`)
      await writeFile(path.join(code, "runner.mjs"), 'globalThis.Bun={plugin(){}}; await import("./index.js");')
      const child = spawn(process.execPath, [path.join(code, "runner.mjs"), ...args], { stdio: ["ignore", "pipe", "pipe"] })
      let stdout = "", stderr = ""
      child.stdout.on("data", chunk => { stdout += chunk })
      child.stderr.on("data", chunk => { stderr += chunk })
      const [exit] = await once(child, "close")
      assert.equal(exit, 0, stderr)
      assert.equal(stdout, `admitted:${args.join(" ")}`)
    } finally { await rm(root, { recursive: true, force: true }) }
  })
}

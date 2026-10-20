import assert from "node:assert/strict"
import { createHash, randomBytes } from "node:crypto"
import { mkdtemp, readFile, readdir, rm, writeFile } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"
import test from "node:test"
import type { WebSocket } from "ws"
import { runMachineCommand } from "./machine-command.js"
import { fakeMachineFiles, serveMachineFiles, startFakeKernel, type FakeKernel, type FakeKernelFrame, type FakeMachineFiles } from "./fake-unix-kernel.test-support.js"

const MIB = 1024 * 1024
const sha256 = (bytes: Buffer) => createHash("sha256").update(bytes).digest("hex")

// Runs the command as an agent would and returns what it printed and how it ended.
async function chariox(argv: string[]) {
  const stdout: string[] = []
  const stderr: string[] = []
  const log = console.log
  const write = process.stderr.write
  console.log = (line: string) => { stdout.push(line) }
  process.stderr.write = ((chunk: string) => { stderr.push(chunk); return true }) as typeof process.stderr.write
  process.exitCode = undefined
  try { await runMachineCommand(argv) } finally {
    console.log = log
    process.stderr.write = write
  }
  const exit = process.exitCode ?? 0
  process.exitCode = undefined
  return { stdout: stdout.join("\n"), stderr: stderr.join(""), exit }
}

async function withRig(run: (rig: { kernel: FakeKernel; store: FakeMachineFiles; dir: string; socket: string[] }) => Promise<void>, wrap?: (serve: ReturnType<typeof serveMachineFiles>) => Parameters<typeof startFakeKernel>[0]) {
  const store = fakeMachineFiles()
  const serve = serveMachineFiles(store)
  const kernel = await startFakeKernel(wrap ? wrap(serve) : serve)
  const dir = await mkdtemp(join(tmpdir(), "chx-files-"))
  try { await run({ kernel, store, dir, socket: ["--timeout", "5", "--socket", kernel.path] }) } finally {
    await kernel.close()
    await rm(dir, { recursive: true, force: true })
  }
}

const requestsOf = (kernel: FakeKernel, name: string) => kernel.frames.filter((frame: FakeKernelFrame) => name in frame.request).map(frame => frame.request[name] as Record<string, unknown>)

test("put sends 1 MiB chunks in order on one connection, each carrying the total size and sha256", async () => {
  await withRig(async ({ kernel, store, dir, socket }) => {
    const content = randomBytes(2 * MIB + MIB / 2)
    await writeFile(join(dir, "big.bin"), content)
    const result = await chariox(["files", "put", ...socket, join(dir, "big.bin"), "/srv/big.bin"])
    assert.equal(result.exit, 0, result.stderr)
    assert.deepEqual(store.files.get("/srv/big.bin"), content)
    const writes = requestsOf(kernel, "WriteMachineFile")
    assert.deepEqual(writes.map(write => write.offset), [0, MIB, 2 * MIB])
    for (const write of writes) assert.deepEqual([write.total_size, write.sha256, write.overwrite], [content.length, sha256(content), false])
    const printed = JSON.parse(result.stdout)
    assert.deepEqual([printed.path, printed.size, printed.sha256, printed.bytes_sent_or_received, printed.resumed_from], ["/srv/big.bin", content.length, sha256(content), content.length, 0])
    assert.equal(typeof printed.elapsed_ms, "number")
    assert.equal(store.sockets.size, 1, "one connection for the whole transfer")
  })
})

test("put continues at the partial size the target reports and starts over when that partial cannot belong to this file", async () => {
  await withRig(async ({ kernel, store, dir, socket }) => {
    const content = randomBytes(2 * MIB + 10)
    await writeFile(join(dir, "f.bin"), content)
    store.partials.set("/srv/f.bin", content.subarray(0, MIB + 7))
    const resumed = await chariox(["files", "put", ...socket, join(dir, "f.bin"), "/srv/f.bin"])
    assert.equal(resumed.exit, 0, resumed.stderr)
    assert.deepEqual(requestsOf(kernel, "WriteMachineFile").map(write => write.offset), [MIB + 7, 2 * MIB + 7])
    assert.deepEqual(store.files.get("/srv/f.bin"), content)
    assert.equal(JSON.parse(resumed.stdout).resumed_from, MIB + 7)
    assert.equal(JSON.parse(resumed.stdout).bytes_sent_or_received, content.length - MIB - 7)

    const before = kernel.frames.length
    store.partials.set("/srv/g.bin", randomBytes(content.length + 1))
    const restarted = await chariox(["files", "put", ...socket, join(dir, "f.bin"), "/srv/g.bin"])
    assert.equal(restarted.exit, 0, restarted.stderr)
    assert.deepEqual(kernel.frames.slice(before).flatMap(frame => "WriteMachineFile" in frame.request ? [(frame.request.WriteMachineFile as { offset: number }).offset] : []), [0, MIB, 2 * MIB])
    assert.deepEqual(store.files.get("/srv/g.bin"), content)
  })
})

test("put passes the kernel's refusal through with its code and needs --overwrite to replace", async () => {
  await withRig(async ({ store, dir, socket }) => {
    await writeFile(join(dir, "f.txt"), "new")
    store.files.set("/srv/f.txt", Buffer.from("old"))
    const refused = await chariox(["files", "put", ...socket, join(dir, "f.txt"), "/srv/f.txt"])
    assert.equal(refused.exit, 1)
    assert.equal(JSON.parse(refused.stderr).error.code, "file_exists")
    assert.match(JSON.parse(refused.stderr).error.message, /send overwrite/)
    assert.equal(store.files.get("/srv/f.txt")?.toString(), "old")
    const replaced = await chariox(["files", "put", ...socket, "--overwrite", join(dir, "f.txt"), "/srv/f.txt"])
    assert.equal(replaced.exit, 0, replaced.stderr)
    assert.equal(store.files.get("/srv/f.txt")?.toString(), "new")
  })
})

test("put names a local file that is missing or not a file", async () => {
  await withRig(async ({ kernel, dir, socket }) => {
    for (const [local, code] of [[join(dir, "nope"), "file_not_found"], [dir, "file_not_a_file"]] as const) {
      const result = await chariox(["files", "put", ...socket, local, "/srv/x"])
      assert.equal(JSON.parse(result.stderr).error.code, code)
    }
    assert.equal(kernel.frames.length, 0)
  })
})

test("get downloads in 1 MiB chunks, verifies the sha256 of what it wrote and leaves no partial file", async () => {
  await withRig(async ({ kernel, store, dir, socket }) => {
    const content = randomBytes(2 * MIB + MIB / 2)
    store.files.set("/srv/big.bin", content)
    const result = await chariox(["files", "get", ...socket, "/srv/big.bin", join(dir, "big.bin")])
    assert.equal(result.exit, 0, result.stderr)
    assert.deepEqual(await readFile(join(dir, "big.bin")), content)
    assert.deepEqual(await readdir(dir), ["big.bin"])
    assert.deepEqual(requestsOf(kernel, "StatMachineFile"), [{ path: "/srv/big.bin", with_sha256: true }])
    assert.deepEqual(requestsOf(kernel, "ReadMachineFile").map(read => [read.offset, read.length]), [[0, MIB], [MIB, MIB], [2 * MIB, MIB / 2]])
    const printed = JSON.parse(result.stdout)
    assert.deepEqual([printed.path, printed.size, printed.sha256, printed.bytes_sent_or_received, printed.resumed_from], [join(dir, "big.bin"), content.length, sha256(content), content.length, 0])
    assert.equal(store.sockets.size, 1, "one connection for the whole transfer")
  })
})

test("get refuses an existing local file unless --overwrite and asks nothing first", async () => {
  await withRig(async ({ kernel, store, dir, socket }) => {
    store.files.set("/srv/f.txt", Buffer.from("remote"))
    await writeFile(join(dir, "f.txt"), "local")
    const refused = await chariox(["files", "get", ...socket, "/srv/f.txt", join(dir, "f.txt")])
    assert.equal(JSON.parse(refused.stderr).error.code, "file_exists")
    assert.match(JSON.parse(refused.stderr).error.message, /--overwrite/)
    assert.equal(await readFile(join(dir, "f.txt"), "utf8"), "local")
    assert.equal(kernel.frames.length, 0)
    const replaced = await chariox(["files", "get", ...socket, "--overwrite", "/srv/f.txt", join(dir, "f.txt")])
    assert.equal(replaced.exit, 0, replaced.stderr)
    assert.equal(await readFile(join(dir, "f.txt"), "utf8"), "remote")
  })
})

test("get keeps no final file and no partial when the bytes do not match the source's sha256", async () => {
  // A kernel that flips one bit of every chunk it sends.
  const corrupting = (serve: ReturnType<typeof serveMachineFiles>) => (socket: WebSocket, frame: FakeKernelFrame) => {
    const send = socket.send.bind(socket)
    socket.send = ((text: string) => {
      const message = JSON.parse(text)
      const chunk = message.response?.MachineFileChunk
      if (chunk) {
        const bytes = Buffer.from(chunk.data_base64, "base64")
        bytes[0] = bytes[0]! ^ 1
        chunk.data_base64 = bytes.toString("base64")
      }
      send(JSON.stringify(message))
    }) as typeof socket.send
    serve(socket, frame)
  }
  await withRig(async ({ store, dir, socket }) => {
    store.files.set("/srv/f.bin", randomBytes(MIB + 3))
    const result = await chariox(["files", "get", ...socket, "/srv/f.bin", join(dir, "f.bin")])
    assert.equal(JSON.parse(result.stderr).error.code, "file_hash_mismatch")
    assert.equal(result.exit, 1)
    assert.deepEqual(await readdir(dir), [])
  }, corrupting)
})

test("get resumes an interrupted download from its .chariox-partial file", async () => {
  await withRig(async ({ kernel, store, dir, socket }) => {
    const content = randomBytes(2 * MIB + 10)
    store.files.set("/srv/f.bin", content)
    await writeFile(join(dir, "f.bin.chariox-partial"), content.subarray(0, MIB + 5))
    const result = await chariox(["files", "get", ...socket, "/srv/f.bin", join(dir, "f.bin")])
    assert.equal(result.exit, 0, result.stderr)
    assert.deepEqual(requestsOf(kernel, "ReadMachineFile").map(read => read.offset), [MIB + 5, 2 * MIB + 5])
    assert.deepEqual(await readFile(join(dir, "f.bin")), content)
    assert.deepEqual(await readdir(dir), ["f.bin"])
    assert.equal(JSON.parse(result.stdout).resumed_from, MIB + 5)
    assert.equal(JSON.parse(result.stdout).bytes_sent_or_received, content.length - MIB - 5)
  })
})

test("stat and ls print the kernel's objects and ask for the sha256 only when told to", async () => {
  await withRig(async ({ kernel, store, socket }) => {
    store.files.set("/srv/a.txt", Buffer.from("hello"))
    const plain = JSON.parse((await chariox(["files", "stat", ...socket, "/srv/a.txt"])).stdout)
    assert.deepEqual(plain, { path: "/srv/a.txt", exists: true, kind: "file", size: 5, modified_ms: 1, sha256: null, partial_size: null })
    const hashed = JSON.parse((await chariox(["files", "stat", ...socket, "--sha256", "/srv/a.txt"])).stdout)
    assert.equal(hashed.sha256, sha256(Buffer.from("hello")))
    assert.deepEqual(requestsOf(kernel, "StatMachineFile").map(stat => stat.with_sha256), [false, true])
    const listing = JSON.parse((await chariox(["files", "ls", ...socket, "/srv"])).stdout)
    assert.deepEqual(listing, { path: "/srv", entries: [{ name: "a.txt", kind: "file", size: 5, modified_ms: 1 }], truncated: false })
  })
})

test("a kernel error on stat arrives with its code", async () => {
  await withRig(async ({ socket }) => {
    const result = await chariox(["files", "get", ...socket, "/srv/missing", "/tmp/never-written"])
    assert.equal(JSON.parse(result.stderr).error.code, "file_not_found")
  })
})

test("--machine goes through the gateway check and tags the requests", async () => {
  await withRig(async ({ kernel, store, socket }) => {
    store.files.set("/srv/a.txt", Buffer.from("hello"))
    const result = await chariox(["files", "stat", ...socket, "--machine", "b3", "/srv/a.txt"])
    assert.equal(result.exit, 0, result.stderr)
    assert.deepEqual(kernel.frames.map(frame => [Object.keys(frame.request)[0], frame.machine]), [["RelayStatus", undefined], ["StatMachineFile", "b3"]])
  })
})

test("a wrong command line is a usage error before anything connects", async () => {
  const nowhere = ["--socket", "/nonexistent/k.sock"]
  for (const argv of [
    ["files"], ["files", "bogus"], ["files", "stat", ...nowhere], ["files", "stat", ...nowhere, "a", "b"], ["files", "ls", ...nowhere, "--sha256", "/p"],
    ["files", "get", ...nowhere, "/remote"], ["files", "put", ...nowhere, "/a", "/b", "/c"], ["files", "put", ...nowhere, "--machine"],
  ]) {
    const result = await chariox(argv)
    assert.equal(JSON.parse(result.stderr).error.code, "usage", argv.join(" "))
    assert.equal(result.exit, 2, argv.join(" "))
  }
})

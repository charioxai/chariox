import assert from "node:assert/strict"
import { createHash, randomBytes } from "node:crypto"
import { chmod, chown, mkdtemp, readFile, readdir, rm, stat, symlink, writeFile } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"
import test from "node:test"
import type { WebSocket } from "ws"
import { runKernelsCommand } from "./kernels-command.js"
import { fakeKernelFiles, serveKernelFiles, startFakeKernel, type FakeKernel, type FakeKernelFrame, type FakeKernelFiles } from "./fake-unix-kernel.test-support.js"

const CHUNK = 256 * 1024
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
  try { await runKernelsCommand(argv) } finally {
    console.log = log
    process.stderr.write = write
  }
  const exit = process.exitCode ?? 0
  process.exitCode = undefined
  return { stdout: stdout.join("\n"), stderr: stderr.join(""), exit }
}

async function withRig(run: (rig: { kernel: FakeKernel; store: FakeKernelFiles; dir: string; socket: string[] }) => Promise<void>, wrap?: (serve: ReturnType<typeof serveKernelFiles>) => Parameters<typeof startFakeKernel>[0]) {
  const store = fakeKernelFiles()
  const serve = serveKernelFiles(store)
  const kernel = await startFakeKernel(wrap ? wrap(serve) : serve)
  const dir = await mkdtemp(join(tmpdir(), "chx-files-"))
  try { await run({ kernel, store, dir, socket: ["--timeout", "5", "--socket", kernel.path] }) } finally {
    await kernel.close()
    await rm(dir, { recursive: true, force: true })
  }
}

const requestsOf = (kernel: FakeKernel, name: string) => kernel.frames.filter((frame: FakeKernelFrame) => name in frame.request).map(frame => frame.request[name] as Record<string, unknown>)

test("put sends 256 KiB chunks in order on one connection, each carrying the total size and sha256", async () => {
  await withRig(async ({ kernel, store, dir, socket }) => {
    const content = randomBytes(2 * CHUNK + CHUNK / 2)
    await writeFile(join(dir, "big.bin"), content)
    const result = await chariox(["files", "put", ...socket, join(dir, "big.bin"), "/srv/big.bin"])
    assert.equal(result.exit, 0, result.stderr)
    assert.deepEqual(store.files.get("/srv/big.bin"), content)
    const writes = requestsOf(kernel, "WriteKernelFile")
    assert.deepEqual(writes.map(write => write.offset), [0, CHUNK, 2 * CHUNK])
    assert.deepEqual(requestsOf(kernel, "ReadKernelFile"), [{ path: "/srv/big.bin.chariox-partial", offset: 0, length: 0 }], "the resume probe is a zero-length read of the staging name")
    for (const write of writes) assert.deepEqual([write.total_size, write.sha256, write.overwrite], [content.length, sha256(content), false])
    const printed = JSON.parse(result.stdout)
    assert.deepEqual([printed.path, printed.size, printed.sha256, printed.bytes_sent_or_received, printed.resumed_from], ["/srv/big.bin", content.length, sha256(content), content.length, 0])
    assert.equal(typeof printed.elapsed_ms, "number")
    assert.equal(store.sockets.size, 1, "one connection for the whole transfer")
  })
})

test("put continues at the partial size the target reports and starts over when that partial cannot belong to this file", async () => {
  await withRig(async ({ kernel, store, dir, socket }) => {
    const content = randomBytes(2 * CHUNK + 10)
    await writeFile(join(dir, "f.bin"), content)
    store.partials.set("/srv/f.bin", content.subarray(0, CHUNK + 7))
    const resumed = await chariox(["files", "put", ...socket, join(dir, "f.bin"), "/srv/f.bin"])
    assert.equal(resumed.exit, 0, resumed.stderr)
    assert.deepEqual(requestsOf(kernel, "WriteKernelFile").map(write => write.offset), [CHUNK + 7, 2 * CHUNK + 7])
    assert.deepEqual(store.files.get("/srv/f.bin"), content)
    assert.equal(JSON.parse(resumed.stdout).resumed_from, CHUNK + 7)
    assert.equal(JSON.parse(resumed.stdout).bytes_sent_or_received, content.length - CHUNK - 7)

    const before = kernel.frames.length
    store.partials.set("/srv/g.bin", randomBytes(content.length + 1))
    const restarted = await chariox(["files", "put", ...socket, join(dir, "f.bin"), "/srv/g.bin"])
    assert.equal(restarted.exit, 0, restarted.stderr)
    assert.deepEqual(kernel.frames.slice(before).flatMap(frame => "WriteKernelFile" in frame.request ? [(frame.request.WriteKernelFile as { offset: number }).offset] : []), [0, CHUNK, 2 * CHUNK])
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

test("get downloads in 256 KiB chunks, prints the sha256 of what it wrote and leaves no partial file", async () => {
  await withRig(async ({ kernel, store, dir, socket }) => {
    const content = randomBytes(2 * CHUNK + CHUNK / 2)
    store.files.set("/srv/big.bin", content)
    const result = await chariox(["files", "get", ...socket, "/srv/big.bin", join(dir, "big.bin")])
    assert.equal(result.exit, 0, result.stderr)
    assert.deepEqual(await readFile(join(dir, "big.bin")), content)
    assert.deepEqual(await readdir(dir), ["big.bin"])
    assert.deepEqual(requestsOf(kernel, "ReadKernelFile").map(read => [read.offset, read.length]), [[0, 0], [0, CHUNK], [CHUNK, CHUNK], [2 * CHUNK, CHUNK / 2]], "the size comes from a zero-length first read")
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

// Runs `during` on the first chunk request, while the download is half done.
const duringDownload = (during: () => Promise<void>) => (serve: ReturnType<typeof serveKernelFiles>) => {
  let first = true
  return (socket: WebSocket, frame: FakeKernelFrame) => {
    if (first && "ReadKernelFile" in frame.request && (frame.request.ReadKernelFile as { length: number }).length > 0) {
      first = false
      void during().then(() => serve(socket, frame))
      return
    }
    serve(socket, frame)
  }
}

test("get keeps the unfinished download private and a replaced file keeps its own permissions", async () => {
  let dir = ""
  let partialMode = 0
  await withRig(async ({ store, dir: rigDir, socket }) => {
    dir = rigDir
    store.files.set("/srv/f.txt", Buffer.from("remote"))
    await writeFile(join(dir, "f.txt"), "local")
    await chmod(join(dir, "f.txt"), 0o600)
    const result = await chariox(["files", "get", ...socket, "--overwrite", "/srv/f.txt", join(dir, "f.txt")])
    assert.equal(result.exit, 0, result.stderr)
    assert.equal((await stat(join(dir, "f.txt"))).mode & 0o777, 0o600, "a 0600 file stays 0600 when it is replaced")
  }, duringDownload(async () => { partialMode = (await stat(join(dir, "f.txt.chariox-partial"))).mode & 0o777 }))
  assert.equal(partialMode, 0o600, "nobody else can read the bytes while they arrive")
})

test("get never replaces a file that appeared while it downloaded, unless --overwrite was given", async () => {
  let dir = ""
  await withRig(async ({ store, dir: rigDir, socket }) => {
    dir = rigDir
    store.files.set("/srv/f.txt", Buffer.from("remote"))
    const result = await chariox(["files", "get", ...socket, "/srv/f.txt", join(dir, "f.txt")])
    assert.equal(JSON.parse(result.stderr).error.code, "file_exists")
    assert.equal(await readFile(join(dir, "f.txt"), "utf8"), "someone else's data")
  }, duringDownload(async () => { await writeFile(join(dir, "f.txt"), "someone else's data") }))
})

test("get --sha256 keeps no final file and no partial when the bytes do not match, and publishes when they do", async () => {
  // A kernel that flips one bit of every chunk it sends.
  const corrupting = (serve: ReturnType<typeof serveKernelFiles>) => (socket: WebSocket, frame: FakeKernelFrame) => {
    const send = socket.send.bind(socket)
    socket.send = ((text: string) => {
      const message = JSON.parse(text)
      const chunk = message.response?.KernelFileChunk
      if (chunk && chunk.data_base64) {
        const bytes = Buffer.from(chunk.data_base64, "base64")
        bytes[0] = bytes[0]! ^ 1
        chunk.data_base64 = bytes.toString("base64")
      }
      send(JSON.stringify(message))
    }) as typeof socket.send
    serve(socket, frame)
  }
  await withRig(async ({ store, dir, socket }) => {
    const content = randomBytes(CHUNK + 3)
    store.files.set("/srv/f.bin", content)
    const result = await chariox(["files", "get", ...socket, "--sha256", sha256(content).toUpperCase(), "/srv/f.bin", join(dir, "f.bin")])
    assert.equal(JSON.parse(result.stderr).error.code, "file_hash_mismatch")
    assert.equal(result.exit, 1)
    assert.deepEqual(await readdir(dir), [])
    // Without an expected hash the download is published and the printed sha256 says what it holds.
    const unchecked = await chariox(["files", "get", ...socket, "/srv/f.bin", join(dir, "f.bin")])
    assert.equal(unchecked.exit, 0, unchecked.stderr)
    assert.equal(JSON.parse(unchecked.stdout).sha256, sha256(await readFile(join(dir, "f.bin"))))
    assert.notEqual(JSON.parse(unchecked.stdout).sha256, sha256(content))
  }, corrupting)
  await withRig(async ({ store, dir, socket }) => {
    const content = randomBytes(CHUNK + 3)
    store.files.set("/srv/f.bin", content)
    const result = await chariox(["files", "get", ...socket, "--sha256", sha256(content).toUpperCase(), "/srv/f.bin", join(dir, "f.bin")])
    assert.equal(result.exit, 0, result.stderr)
    assert.equal(JSON.parse(result.stdout).sha256, sha256(content))
    assert.deepEqual(await readdir(dir), ["f.bin"])
  })
})

test("get stops when the source changes size under it", async () => {
  const growing = (serve: ReturnType<typeof serveKernelFiles>) => {
    let reads = 0
    return (socket: WebSocket, frame: FakeKernelFrame) => {
      if ("ReadKernelFile" in frame.request && ++reads === 3) {
        const send = socket.send.bind(socket)
        socket.send = ((text: string) => {
          const message = JSON.parse(text)
          message.response.KernelFileChunk.size += 1
          send(JSON.stringify(message))
        }) as typeof socket.send
      }
      serve(socket, frame)
    }
  }
  await withRig(async ({ store, dir, socket }) => {
    store.files.set("/srv/f.bin", randomBytes(2 * CHUNK))
    const result = await chariox(["files", "get", ...socket, "/srv/f.bin", join(dir, "f.bin")])
    assert.equal(JSON.parse(result.stderr).error.code, "file_io_failed")
    assert.match(JSON.parse(result.stderr).error.message, /changed from/)
    assert.deepEqual(await readdir(dir), ["f.bin.chariox-partial"])
  }, growing)
})

test("get resumes an interrupted download from its .chariox-partial file", async () => {
  await withRig(async ({ kernel, store, dir, socket }) => {
    const content = randomBytes(2 * CHUNK + 10)
    store.files.set("/srv/f.bin", content)
    await writeFile(join(dir, "f.bin.chariox-partial"), content.subarray(0, CHUNK + 5), { mode: 0o600 })
    const result = await chariox(["files", "get", ...socket, "/srv/f.bin", join(dir, "f.bin")])
    assert.equal(result.exit, 0, result.stderr)
    assert.deepEqual(requestsOf(kernel, "ReadKernelFile").map(read => read.offset), [0, CHUNK + 5, 2 * CHUNK + 5])
    assert.deepEqual(await readFile(join(dir, "f.bin")), content)
    assert.deepEqual(await readdir(dir), ["f.bin"])
    assert.equal(JSON.parse(result.stdout).resumed_from, CHUNK + 5)
    assert.equal(JSON.parse(result.stdout).bytes_sent_or_received, content.length - CHUNK - 5)
  })
})

// EA2-45: the download's staging name is predictable, so a link or another user's file planted there
// must neither be written through nor have its bytes and owner kept.
test("get never writes through a symlink planted at its staging name", async () => {
  await withRig(async ({ store, dir, socket }) => {
    const content = randomBytes(CHUNK + 10)
    store.files.set("/srv/f.bin", content)
    const victim = join(dir, "victim.txt")
    await writeFile(victim, "private notes")
    await symlink(victim, join(dir, "f.bin.chariox-partial"))
    const result = await chariox(["files", "get", ...socket, "/srv/f.bin", join(dir, "f.bin")])
    assert.equal(result.exit, 0, result.stderr)
    assert.equal(await readFile(victim, "utf8"), "private notes")
    assert.deepEqual(await readFile(join(dir, "f.bin")), content)
    assert.equal(JSON.parse(result.stdout).resumed_from, 0)
    assert.deepEqual((await readdir(dir)).sort(), ["f.bin", "victim.txt"])
  })
})

test("get does not resume onto another user's file or one others can read", { skip: process.getuid?.() !== 0 }, async () => {
  for (const [uid, mode] of [[65534, 0o600], [0, 0o644]] as const) {
    await withRig(async ({ kernel, store, dir, socket }) => {
      const content = randomBytes(CHUNK + 10)
      store.files.set("/srv/f.bin", content)
      const partial = join(dir, "f.bin.chariox-partial")
      await writeFile(partial, content.subarray(0, 100))
      await chown(partial, uid, uid)
      await chmod(partial, mode)
      const result = await chariox(["files", "get", ...socket, "/srv/f.bin", join(dir, "f.bin")])
      assert.equal(result.exit, 0, result.stderr)
      assert.deepEqual(requestsOf(kernel, "ReadKernelFile").map(read => read.offset), [0, 0, CHUNK])
      assert.equal((await stat(join(dir, "f.bin"))).uid, 0)
      assert.deepEqual(await readFile(join(dir, "f.bin")), content)
    })
  }
})

test("stat and ls are gone: the CLI points at the terminal", async () => {
  await withRig(async ({ kernel, socket }) => {
    for (const verb of ["stat", "ls"]) {
      const result = await chariox(["files", verb, ...socket, "/srv/a.txt"])
      assert.equal(JSON.parse(result.stderr).error.code, "usage")
      assert.match(JSON.parse(result.stderr).error.message, /files get\|put/)
    }
    assert.equal(kernel.frames.length, 0)
    const guide = await chariox(["guide"])
    assert.doesNotMatch(guide.stdout, /files (stat|ls)|stat\|ls/)
    assert.match(guide.stdout, /Listing, stat, hashing and small reads go through a terminal/)
  })
})

test("a kernel error on the first read arrives with its code", async () => {
  await withRig(async ({ socket }) => {
    const result = await chariox(["files", "get", ...socket, "/srv/missing", "/tmp/never-written"])
    assert.equal(JSON.parse(result.stderr).error.code, "file_not_found")
  })
})

test("--kernel goes through the gateway check and tags the requests", async () => {
  await withRig(async ({ kernel, store, dir, socket }) => {
    store.files.set("/srv/a.txt", Buffer.from("hello"))
    const result = await chariox(["files", "get", ...socket, "--kernel", "b3", "/srv/a.txt", join(dir, "a.txt")])
    assert.equal(result.exit, 0, result.stderr)
    assert.deepEqual(kernel.frames.map(frame => [Object.keys(frame.request)[0], frame.kernel]), [["RelayStatus", undefined], ["ReadKernelFile", "b3"], ["ReadKernelFile", "b3"]])
  })
})

test("a wrong command line is a usage error before anything connects", async () => {
  const nowhere = ["--socket", "/nonexistent/k.sock"]
  for (const argv of [
    ["files"], ["files", "bogus"], ["files", "stat", ...nowhere, "/p"], ["files", "ls", ...nowhere, "/p"],
    ["files", "get", ...nowhere, "--sha256", "not-hex", "/a", "/b"], ["files", "get", ...nowhere, "--sha256", "/a", "/b"], ["files", "put", ...nowhere, "--sha256", "a".repeat(64), "/a", "/b"],
    ["files", "get", ...nowhere, "/remote"], ["files", "put", ...nowhere, "/a", "/b", "/c"], ["files", "put", ...nowhere, "--kernel"],
  ]) {
    const result = await chariox(argv)
    assert.equal(JSON.parse(result.stderr).error.code, "usage", argv.join(" "))
    assert.equal(result.exit, 2, argv.join(" "))
  }
})

test("a relative local path means the directory the agent runs in, not the CLI's own", async () => {
  await withRig(async ({ store, dir, socket }) => {
    const before = process.env.CHARIOX_CLI_ORIGINAL_CWD
    process.env.CHARIOX_CLI_ORIGINAL_CWD = dir
    try {
      const content = randomBytes(1000)
      await writeFile(join(dir, "rel.bin"), content)
      const put = await chariox(["files", "put", ...socket, "rel.bin", "/srv/rel.bin"])
      assert.equal(put.exit, 0, put.stderr)
      assert.deepEqual(store.files.get("/srv/rel.bin"), content)
      const get = await chariox(["files", "get", ...socket, "/srv/rel.bin", "back.bin"])
      assert.equal(get.exit, 0, get.stderr)
      assert.deepEqual(await readFile(join(dir, "back.bin")), content)
    } finally {
      if (before === undefined) delete process.env.CHARIOX_CLI_ORIGINAL_CWD
      else process.env.CHARIOX_CLI_ORIGINAL_CWD = before
    }
  })
})

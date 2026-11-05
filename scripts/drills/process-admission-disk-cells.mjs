// The disk cells of the process-admission live drill (OF-37 attribution, OF-38 temp, OF-39 trim, OF-40 ask,
// OF-42 runaway). Each flow gives real agents real work through their own tools, then checks the kernel's
// side on the real machine: files, processes and the session's process rows. Nothing here talks to a provider.
import assert from "node:assert/strict"
import { existsSync, readFileSync, readdirSync, statSync, writeFileSync } from "node:fs"
import { join } from "node:path"
import { setTimeout as delay } from "node:timers/promises"

const short = id => id.replace(/^agent-/, "").replace(/[^A-Za-z0-9]/g, "").slice(0, 8)
const GIB = 1024 ** 3

export async function runDiskCells(ctx) {
  const { scenario, agents, ask, note, until, work, dir, sessionId, rpc, report, flags, transcripts, providerOf } = ctx
  const latestRows = () => ctx.latestRows()
  const checks = (report.checks = [])
  const check = (name, ok, detail = {}) => { checks.push({ name, ok: !!ok, ...detail }); note(ok ? "check-pass" : "check-FAIL", { name, ...detail }) }
  const tempRoot = join(dir, "chariox/t")
  const tempDir = agent => join(tempRoot, short(sessionId), short(agent.id))
  const answer = (tag, n) => join(work, `answer-${tag}-b${n}.txt`)
  const waitAnswers = (tag, timeoutS = 600) => Promise.all(agents.map((_, i) =>
    until(() => existsSync(answer(tag, i + 1)) && /DONE/.test(readFileSync(answer(tag, i + 1), "utf8")), `agent ${i + 1} answer ${tag}`, timeoutS * 1000)
      .then(() => note("answered", { agent: i + 1, provider: providerOf(i + 1), tag }))))
  const say = (template, tag) => Promise.all(agents.map((target, i) => ask(template.replaceAll("{N}", String(i + 1)), target)))
  const df = () => { const [total, free] = ctx.execFileSync("df", ["-B1", "--output=size,avail", work], { encoding: "utf8" }).trim().split("\n")[1].trim().split(/\s+/).map(Number); return { total, free } }
  const hogs = () => ctx.osProcesses("hog.py")
  const rowsOf = agent => latestRows().filter(p => p.owner?.id === agent.id)

  if (scenario === "temp") {
    // OF-38: no instruction about where scratch goes; the tools find TMPDIR set by the kernel.
    const prompt = `Run exactly this one shell command in the current directory and nothing else: sh -c 'f=answer-temp-b{N}.txt; echo "TMPDIR=$TMPDIR TMP=$TMP TEMP=$TEMP" > $f; echo "mktemp=$(mktemp)" >> $f; python3 -c "import tempfile;print(\\"python=\\"+tempfile.mkdtemp())" >> $f; node -e "console.log(\\"node=\\"+require(\\"os\\").tmpdir())" >> $f 2>/dev/null; echo DONE >> $f'. Afterwards append one more line to the same file, starting with INSTR= , that quotes word for word the sentence in your own instructions that mentions temporary files (INSTR=NONE if there is none).`
    await say(prompt, "temp")
    await waitAnswers("temp")
    // The agents append the quoted sentence after the shell command is done.
    await Promise.all(agents.map((_, i) => until(() => /^INSTR=/m.test(readFileSync(answer("temp", i + 1), "utf8")), `agent ${i + 1} quote`, 120000).catch(() => note("no-quote", { agent: i + 1 }))))
    for (const [i, target] of agents.entries()) {
      const n = i + 1, lines = Object.fromEntries(readFileSync(answer("temp", n), "utf8").split("\n").filter(l => l.includes("=")).map(l => [l.slice(0, l.indexOf("=")), l.slice(l.indexOf("=") + 1)]))
      const expected = tempDir(target)
      const tag = `${providerOf(n)}`
      check(`${tag}: TMPDIR is its own folder`, lines.TMPDIR?.includes(`TMPDIR=${expected}`) || lines.TMPDIR?.startsWith(expected), { got: lines.TMPDIR, expected })
      check(`${tag}: TMP and TEMP are the same`, new Set([lines.TMPDIR?.split(" ")[0], lines.TMPDIR?.match(/TMP=(\S+)/)?.[1], lines.TMPDIR?.match(/TEMP=(\S+)/)?.[1]].filter(Boolean)).size <= 2)
      check(`${tag}: mktemp lands in it`, lines.mktemp?.startsWith(expected), { got: lines.mktemp })
      check(`${tag}: python tempfile lands in it`, lines.python?.startsWith(expected), { got: lines.python })
      check(`${tag}: the path is short (sockets under 107 bytes)`, expected.length + "/SingletonSocket-12345".length < 107, { length: expected.length })
      check(`${tag}: the folder is private`, (statSync(expected).mode & 0o777) === 0o700, { mode: (statSync(expected).mode & 0o777).toString(8) })
      check(`${tag}: its instructions carry the temp line`, /TMPDIR/.test(lines.INSTR ?? "") && /cleared when idle/.test(lines.INSTR ?? ""), { instr: lines.INSTR })
      report[`temp_${n}`] = { provider: tag, lines }
    }
  }

  if (scenario === "trim-live") {
    // Odd agents first use an inherited empty TMPDIR after a sweep; even
    // agents keep a real listener open while stale siblings are removed.
    writeFileSync(join(work, "trim-live.py"), `import os, pathlib, socket, sys, tempfile, time
n = int(sys.argv[1])
root = pathlib.Path(os.environ["TMPDIR"])
answer = pathlib.Path.cwd() / ("answer-trim-live-b%d.txt" % n)
try:
    listener = None
    if n % 2 == 0:
        listener = socket.socket(socket.AF_UNIX)
        listener.bind(str(root / "live.sock"))
        listener.listen(1)
        listener.settimeout(5)
    stale = root / "stale.txt"
    stale.write_text("disposable")
    deadline = time.monotonic() + 180
    while stale.exists() and time.monotonic() < deadline:
        time.sleep(1)
    assert not stale.exists(), "the sweep did not run"
    if listener:
        client = socket.socket(socket.AF_UNIX)
        client.connect(str(root / "live.sock"))
        connection, _ = listener.accept()
        connection.close()
        client.close()
        listener.close()
    else:
        fd, path = tempfile.mkstemp(dir=root)
        os.close(fd)
        os.unlink(path)
    answer.write_text("PASS DONE")
except Exception as error:
    answer.write_text("FAIL %s DONE" % error)
`)
    await say("Run exactly python3 trim-live.py {N} in the current directory. Report its result without changing the script or retrying it.", "trim-live")
    await waitAnswers("trim-live", 240)
    for (const [i] of agents.entries()) {
      const result = readFileSync(answer("trim-live", i + 1), "utf8")
      check(`${providerOf(i + 1)} ${i + 1}: ${i % 2 ? "bound socket accepts after sweep" : "inherited TMPDIR accepts its first later use"}`, result === "PASS DONE", { result })
    }
    assert(checks.every(c => c.ok), "the live scratch regression failed")
  }

  if (scenario === "trim") {
    // OF-39: stale scratch goes; an open file, a busy folder and a freshly written item stay; the ended Room clears.
    const prompt = `Run exactly this one shell command and nothing else: sh -c 'W=$PWD; cd "$TMPDIR" && echo old > stale.txt && mkdir -p busy && (cd busy && setsid nohup sleep 3000 > /dev/null 2>&1 &) ; (setsid nohup python3 -c "import time; f=open(\\"open.txt\\",\\"w\\"); f.write(\\"x\\"); f.flush(); time.sleep(3000)" > /dev/null 2>&1 &) ; echo DONE > $W/answer-trim-b{N}.txt'`
    await say(prompt, "trim")
    await waitAnswers("trim")
    const idle = Number(flags.get("--temp-idle-s") ?? 20)
    const folders = agents.map(tempDir)
    for (const folder of folders) assert(existsSync(join(folder, "stale.txt")), `${folder} was not prepared`)
    // What holds the folders at this moment: the checks below mean nothing if the agent's tool ended these processes.
    const alive = readdirSync("/proc").filter(p => /^\d+$/.test(p)).flatMap(p => { try { const cwd = ctx.execFileSync("readlink", [`/proc/${p}/cwd`], { encoding: "utf8" }).trim(); const open = readdirSync(`/proc/${p}/fd`).map(fd => { try { return ctx.execFileSync("readlink", [`/proc/${p}/fd/${fd}`], { encoding: "utf8" }).trim() } catch { return "" } }); return folders.some(f => cwd.startsWith(f) || open.some(o => o.startsWith(f))) ? [{ pid: Number(p), cwd, cmd: readFileSync(`/proc/${p}/cmdline`, "utf8").split("\0").slice(0, 3).join(" ") }] : [] } catch { return [] } })
    note("trim-prepared", { idle_s: idle, folders, holders: alive })
    check("the agents' processes hold the open file and the busy folder", folders.every(folder => alive.some(a => a.cwd.startsWith(join(folder, "busy"))) && existsSync(join(folder, "open.txt"))), { alive: alive.length })
    const started = Date.now()
    let goneAt = null
    while (Date.now() - started < (idle + 150) * 1000) {
      // Something writes into every folder all the time: its item is never idle.
      for (const folder of folders) writeFileSync(join(folder, "fresh.txt"), String(Date.now()))
      const listing = folders.map(folder => existsSync(folder) ? readdirSync(folder).sort().join(",") : "(folder gone)")
      if (listing.every(l => !l.includes("stale.txt")) && goneAt === null) goneAt = (Date.now() - started) / 1000
      if (goneAt !== null && Date.now() - started > (goneAt + 5) * 1000) break
      await delay(3000)
    }
    for (const [i, folder] of folders.entries()) {
      const names = existsSync(folder) ? readdirSync(folder) : []
      const tag = providerOf(i + 1)
      check(`${tag}: the stale item was cleared`, !names.includes("stale.txt"), { after_s: goneAt, names })
      check(`${tag}: the open file stays`, names.includes("open.txt"), { names })
      check(`${tag}: the busy folder stays`, names.includes("busy"), { names })
      check(`${tag}: a freshly written item stays`, names.includes("fresh.txt"), { names })
    }
    // The Room ends: its scratch goes at once (what was open goes with the processes that held it).
    const room = join(tempRoot, short(sessionId))
    await rpc({ EndSession: { session_id: sessionId } })
    await until(() => !existsSync(room), "the ended Room's temp folder to go", 20000).then(() => check("ending the Room clears its temp at once", true), () => check("ending the Room clears its temp at once", false, { left: existsSync(room) ? readdirSync(room) : [] }))
  }

  if (scenario === "ask" || scenario === "runaway") {
    const before = df()
    note("volume", { ...before, floor_gib: Math.max(before.total / 20, 10 * GIB) / GIB })
    report.volume = before
  }

  if (scenario === "ask") {
    // OF-40/OF-37: every agent leaves an orphan writing into a deleted file and a build output; the disk falls below
    // its floor; the kernel asks the biggest holders, naming what they hold; the agents free it and report.
    const prompt = `You work in the current folder. Run these shell commands one after another, then reply DONE_ask_b{N}:
1. setsid nohup python3 hog.py b{N} 6 1500 > hog-b{N}.log 2>&1 &
2. mkdir -p target-b{N} && (cd target-b{N} && python3 ../write.py b{N} 900 100)   (target-b{N} is your build output: python3 ../write.py regenerates it whenever you need it)
3. echo DONE > answer-ask-b{N}.txt
If the system messages you about disk space later, act on it.`
    const sawRow = new Set()
    const watching = { stop: false }
    const watch = (async () => {
      while (!watching.stop) {
        for (const target of agents) for (const p of rowsOf(target)) if (p.deleted_open_bytes > 0 && !sawRow.has(target.id + Math.floor(p.deleted_open_bytes / 2 ** 27))) {
          sawRow.add(target.id + Math.floor(p.deleted_open_bytes / 2 ** 27))
          note("hog-row", { agent: target.id, pid: p.pid, written_mb: Math.round((p.disk_written_bytes ?? 0) / 2 ** 20), deleted_open_mb: Math.round(p.deleted_open_bytes / 2 ** 20), outlived: p.outlived, state: p.state?.kind })
        }
        await delay(1000)
      }
    })()
    const started = Date.now()
    await say(prompt, "ask")
    await waitAnswers("ask", 900)
    note("setup-done", { seconds: (Date.now() - started) / 1000, ...df() })
    // The kernel asks within seconds of the disk falling below its floor; the agents then free their output.
    const freed = await until(() => agents.every((_, i) => !existsSync(join(work, `target-b${i + 1}`))) && hogs().length === 0, "the agents to free their output and stop their leftovers", 600000).then(() => true, () => false)
    check("agents freed their build output and stopped the orphan writers after the ask", freed, { hogs_left: hogs().map(h => h.pid), targets_left: readdirSync(work).filter(n => n.startsWith("target-")) })
    const text = transcripts()
    check("the ask named the biggest folders and the process holding deleted files", /deleted files/.test(text) && /target-b\d/.test(text) || providerOf(1) !== "claude", { note: "Claude transcripts" })
    report.volume_after = df()
    for (const target of agents) report[`rows_${target.id}`] = rowsOf(target).map(p => ({ pid: p.pid, command: p.command, written: p.disk_written_bytes, deleted_open: p.deleted_open_bytes, outlived: p.outlived }))
    watching.stop = true
    await watch
    for (const n of agents.keys()) check(`agent ${n + 1} (${providerOf(n + 1)}) attributed: an orphan filling a deleted file was shown on its row`, [...sawRow].some(key => key.startsWith(agents[n].id)))
  }

  if (scenario === "runaway") {
    // OF-42/OF-37: agent 1's orphan writes 32 MB/s into a deleted file while the others build. The kernel finds
    // it, attributes it to its owner and ends it (never pauses); the other owners' builds are never paused.
    const hogPrompt = `You work in the current folder. Run these shell commands one after another: first setsid nohup python3 hog.py b{N} 32 9000 > hog-b{N}.log 2>&1 &  then run python3 -c 'import time; time.sleep(330)' (in the foreground, as one command, with a timeout of 400000 ms), then echo DONE > answer-run-b{N}.txt`
    const buildPrompt = `You work in the current folder. Start this build in the background: setsid nohup sh -c 'for i in 1 2 3 4 5 6 7 8; do rm -rf t-b{N}; mkdir t-b{N}; (cd t-b{N} && python3 ../write.py b{N}-$i 400 40) || exit 1; done; echo DONE > answer-run-b{N}.txt' > build-b{N}.log 2>&1 &  Then wait for it by running this one shell command: until [ -f answer-run-b{N}.txt ]; do sleep 5; done; echo finished (give that command a timeout of 600000 ms), and reply with the word finished.`
    const paused = new Set(), held = new Set()
    let least = df().free
    const watching = { stop: false }
    const watch = (async () => {
      while (!watching.stop) {
        for (const p of latestRows()) if (p.state?.kind === "paused" || p.state?.kind === "held") (p.state.kind === "paused" ? paused : held).add(`${p.state.kind}:${p.pid}:${(p.state.command_line ?? p.command).slice(0, 40)}`)
        least = Math.min(least, df().free)
        for (const target of agents) for (const p of rowsOf(target)) if (p.deleted_open_bytes > 0 && Date.now() % 5 < 1) note("hog-row", { agent: target.id, pid: p.pid, written_mb: Math.round((p.disk_written_bytes ?? 0) / 2 ** 20), deleted_open_mb: Math.round(p.deleted_open_bytes / 2 ** 20), outlived: p.outlived, state: p.state?.kind })
        await delay(500)
      }
    })()
    const started = Date.now()
    await Promise.all(agents.map((target, i) => ask((i === 0 ? hogPrompt : buildPrompt).replaceAll("{N}", String(i + 1)), target)))
    let hogSeen = null
    const killed = await until(() => { const h = hogs(); if (h.length && !hogSeen) hogSeen = Date.now(); return hogSeen && h.length === 0 }, "the filler to be stopped", 480000).then(() => (Date.now() - started) / 1000, () => null)
    check("the orphan filling the disk was stopped", killed !== null, { after_s: killed })
    const log = existsSync(join(work, "hog-b1.log")) ? readFileSync(join(work, "hog-b1.log"), "utf8") : ""
    check("it was told why, on its own error stream", /was filling it/.test(log), { log: log.slice(0, 400) })
    const buildsDone = agents.slice(1).map((_, i) => until(() => existsSync(answer("run", i + 2)), `build ${i + 2}`, 600000).then(() => true, () => false))
    const finished = await Promise.all(buildsDone)
    check("the other owners' builds finished", finished.every(Boolean), { finished })
    // A build that starts while the disk is below its floor waits in line (held, it has written nothing yet); a running writer is never paused.
    check("no other owner's running build was paused for it", paused.size === 0, { paused: [...paused], held: [...held] })
    check("free disk never reached zero", least > 256 * 2 ** 20, { least_free_mb: Math.round(least / 2 ** 20) })
    await ask("Read hog-b1.log in the current folder and tell me in one sentence why your background job hog.py ended; write that sentence into answer-why-b1.txt", agents[0])
    await until(() => existsSync(join(work, "answer-why-b1.txt")), "the owner to read why", 600000).then(() => check("the agent read why its job ended", /fill|disk/i.test(readFileSync(join(work, "answer-why-b1.txt"), "utf8")), { answer: readFileSync(join(work, "answer-why-b1.txt"), "utf8").slice(0, 300) }), () => check("the agent read why its job ended", false))
    watching.stop = true
    await watch
    report.paused_rows = [...paused]
    report.held_rows = [...held]
    report.least_free_mb = Math.round(least / 2 ** 20)
  }
  note("scenario-done", { scenario, passed: checks.filter(c => c.ok).length, failed: checks.filter(c => !c.ok).map(c => c.name) })
}

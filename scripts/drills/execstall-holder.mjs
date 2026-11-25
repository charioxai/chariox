// A real access-grant holder exits while a maintenance pass waits on a slow gateway start.
// The pump must stay quiet: a spinning pump burns a core for the rest of the pass.
import assert from "node:assert/strict"
import { spawn } from "node:child_process"
import { existsSync, readFileSync, writeFileSync } from "node:fs"
import { join } from "node:path"
import { setTimeout as delay } from "node:timers/promises"

const CLOCK_TICKS_PER_SECOND = 100
const WINDOW_MS = 3_000

function cpuSeconds(pid) {
  // utime and stime follow the command name, which may contain spaces: count from the closing paren.
  const fields = readFileSync(`/proc/${pid}/stat`, "utf8").split(") ")[1].split(" ")
  return (Number(fields[11]) + Number(fields[12])) / CLOCK_TICKS_PER_SECOND
}

export async function proveDeadHolderDoesNotSpin({ target, room, socket, nodeExecutable, kernelDir, wait, logs, evidence, expected }) {
  // A starved kernel answers nothing: its clients reject. Record that as a symptom, do not die before measuring.
  const clientErrors = []
  process.on("unhandledRejection", error => clientErrors.push(String(error?.message ?? error)))
  const requester = join(kernelDir, "requester.mjs")
  const answer = join(kernelDir, "grant-answer.json")
  writeFileSync(requester, `import { writeFileSync } from "node:fs"
import { LocalIpcClient } from ${JSON.stringify(join(import.meta.dirname, "../../packages/kernel-client/dist/ipc.js"))}
const client = new LocalIpcClient("ws+unix://${socket}", { controlRequestRetryDeadlineMs: 0 })
const result = await client.send({ RequestKernelAccess: { holder_pid: Number(process.argv[2]), term: "until_holder_exits" } })
writeFileSync(${JSON.stringify(answer)}, JSON.stringify(result))
await client.close()
`)
  // The holder is the shell: it stays alive after its requester child has its grant.
  const holder = spawn("/bin/sh", ["-c", `"${nodeExecutable}" "${requester}" "$$"; read line`], { stdio: ["pipe", "ignore", "ignore"] })
  const granted = await wait(() => existsSync(answer) && JSON.parse(readFileSync(answer, "utf8")), "the standing grant")
  assert.equal(granted.KernelAccessGranted?.grant.holder_pid, holder.pid, JSON.stringify(granted))
  const grants = async () => (await target.send({ ListKernelAccessGrants: {} })).KernelAccessGrantsListed.grants
  assert.equal((await grants()).length, 1, "The holder's grant is live")

  // The first gateway start exits after 1.5 s; its relaunch waits 14 s inside the maintenance pass.
  await target.send({ CreateAgentTrigger: {
    session_id: room.session.id, agent_id: room.agent.id, source: { kind: "http" }, prompt: "Report the drill event",
  } })
  const stalled = () => logs(kernelDir).some(row =>
    row.message === "kernel operation stalled" && row.operation === "transport_maintenance" && row.stage === "reconcile agent trigger gateways")
  await wait(stalled, "maintenance to wait on the gateway", 30_000)

  const before = cpuSeconds(target.pid())
  holder.kill("SIGKILL")
  await delay(WINDOW_MS)
  const cpu = cpuSeconds(target.pid()) - before
  const stillWaiting = logs(kernelDir).filter(row => row.message === "kernel operation stalled" && row.operation === "transport_maintenance").length
  const result = { expected, windowMs: WINDOW_MS, kernelCpuSeconds: cpu, maintenanceStallRows: stillWaiting, clientErrors }
  writeFileSync(join(evidence, "holder-result.json"), JSON.stringify(result, null, 2))
  if (expected === "red") assert(cpu > 1, `A spinning pump burns the window: ${JSON.stringify(result)}`)
  else assert(cpu < 0.5 && clientErrors.length === 0, `The pump stays quiet while maintenance waits: ${JSON.stringify(result)}`)
  if (expected === "red") return result
  // The next pass sweeps the dead holder's grant.
  await wait(async () => (await grants()).length === 0, "the dead holder's grant to end", 40_000)
  return result
}

// MP-07 / MP-08 / MP-11: kernel-owned deployment subprocess; secrets only on stdin.
import { runSshMachine } from "./transport.mjs"
let input
try {
  let bytes = Buffer.alloc(0)
  for await (const chunk of process.stdin) {
    if (bytes.length + chunk.length > 32768) throw new Error("oversized input")
    bytes = Buffer.concat([bytes, chunk])
  }
  input = JSON.parse(bytes.toString("utf8")); bytes.fill(0)
  if (input.request.action === "install") await runSshMachine(input.host, input.request, input.release)
  const request = { ...input.request, action: input.request.action === "install" ? "start" : input.request.action }
  const result = await runSshMachine(input.host, request, undefined, { enrollment: input.enrollment })
  process.stdout.write(JSON.stringify(result))
} catch (error) {
  process.stderr.write("MP-07/MP-08/MP-11: SSH deployment failed; check access, signed release, enrollment and user service prerequisites\n")
  process.exitCode = error?.exitCode === 75 ? 75 : 1
} finally { if (input?.enrollment) input.enrollment.ticket = "" }

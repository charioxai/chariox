// MP-11: retain the recorded setsid leader until command and group have settled.
// Its PID/start identity pins the run session across a fast command-parent exit.
import { spawn } from 'node:child_process'
import { processSnapshot } from './owned-process-signals.mjs'
const [command, ...args] = process.argv.slice(2)
if (process.pid <= 1 || !command) process.exit(127)
const child = spawn(command, args, { stdio: 'inherit' })
child.once('error', () => process.exit(127))
child.once('exit', (code, signal) => {
  const timer = setInterval(() => {
    try {
      const remaining = processSnapshot().filter(row => row.pid !== process.pid
        && row.pgid === process.pid && !['Z', 'X'].includes(row.state))
      if (remaining.length) return
      clearInterval(timer)
      process.exit(code ?? (signal ? 1 : 127))
    } catch { /* Inspection failure retains authority; caller owns timeout. */ }
  }, 25)
})

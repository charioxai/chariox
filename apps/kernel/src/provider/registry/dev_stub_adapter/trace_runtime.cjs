const { randomUUID } = require('node:crypto')
const { setTimeout: delay } = require('node:timers/promises')
const { once } = require('node:events')

async function runtimeTool(name, args, signal) {
  const value = await requestRuntime({ id: randomUUID(), method: 'tools/call', params: { name, arguments: args } }, signal)
  if (value.error || value.result?.isError) throw new Error(`runtime refused ${name}`)
  return value.result?.structuredContent ?? value.result
}

async function terminalBurst(lines, lineBytes, rate, signal) {
  const batch = Math.max(1, Math.floor(rate / 20))
  const start = performance.now()
  const line = filler(lineBytes) + '\n'
  for (let emitted = 0; emitted < lines;) {
    const count = Math.min(batch, lines - emitted)
    if (!process.stdout.write(line.repeat(count))) await once(process.stdout, 'drain', { signal })
    emitted += count
    const wait = emitted * 1000 / rate - (performance.now() - start)
    if (wait > 0) await delay(wait, undefined, { signal })
  }
}

async function replaySteps(turn, output, signal) {
  const start = performance.now()
  for (const step of turn.slice(1)) {
    const wait = step.t - (performance.now() - start)
    if (wait > 0) await delay(wait, undefined, { signal })
    signal.throwIfAborted()
    switch (step.kind) {
      case 'reasoning':
      case 'text': {
        const text = filler(step.chars, step.markdown)
        await output.stream(step.kind, text, step.stream_ms, signal)
        break
      }
      case 'tool': {
        const input = { sample: filler(step.input_bytes) }
        const part = output.part('tool', { tool: step.name, state: { status: 'running', input } })
        if (step.ms) await delay(step.ms, undefined, { signal })
        const result = filler(step.output_bytes)
        const state = step.outcome === 'ok'
          ? { status: 'completed', input, output: result }
          : { status: 'error', input, error: result }
        output.update(part, { state })
        break
      }
      case 'terminal':
        await Promise.all([
          terminalBurst(step.lines, step.line_bytes, step.rate_lps, signal),
          runtimeTool('chariox.terminal_open', {
            title: 'Load lab terminal',
            command: ['node', '-e', `const { once } = require('node:events'); const { setTimeout: delay } = require('node:timers/promises'); const filler = ${filler.toString()}; (${terminalBurst.toString()})(${step.lines}, ${step.line_bytes}, ${step.rate_lps}, new AbortController().signal)`],
          }, signal),
        ])
        break
      case 'spawn': {
        const { turns } = loadTrace(step.agent)
        const prompt = filler(turns[0][0].chars)
        await runtimeTool('chariox.room.run_command', {
          command: `agent spawn ${step.agent} --provider dev-stub --model trace --task '${prompt}'`,
        }, signal)
        break
      }
      case 'notify':
        for (let i = 0; i < step.count; i++) {
          await runtimeTool('chariox.notify_person', { title: `${step.source} ${i + 1}`, body: 'Generated load lab notice.' }, signal)
          if (i + 1 < step.count && step.interval_ms) await delay(step.interval_ms, undefined, { signal })
        }
        break
      case 'permission':
        await output.permission(step.tool, signal)
        break
      case 'wait':
        await delay(step.ms, undefined, { signal })
        break
      case 'end':
        return step.outcome
    }
  }
}

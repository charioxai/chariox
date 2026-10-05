// MP-08 / MP-10: durable, exclusive reservation precedes any prompt RPC.
import { mkdir, open } from 'node:fs/promises'
export async function submitOnce({ directory, scope, taskId, runId, submit }) {
  await mkdir(directory, { recursive: true, mode: 0o700 })
  const file = `${directory}/${scope}-${encodeURIComponent(taskId)}.json`
  const handle = await open(file, 'wx', 0o600)
  try {
    await handle.writeFile(JSON.stringify({ mpItems: ['MP-08', 'MP-10'], scope, taskId,
      runId, state: 'reserved', reservedAt: new Date().toISOString() }) + '\n')
    await handle.sync()
  } finally { await handle.close() }
  const parent = await open(directory, 'r')
  try { await parent.sync() } finally { await parent.close() }
  // Never release a reservation on error: the provider may have received the RPC.
  return submit()
}

// MP-08 / MP-10 / MP-11: shared product accounting read, no credential inspection.
import { pathToFileURL } from 'node:url'
import path from 'node:path'
const [runtimeRoot, endpoint, sessionId] = process.argv.slice(2)
const { LocalIpcClient } = await import(pathToFileURL(path.join(runtimeRoot, 'packages/kernel-client/dist/ipc.js')))
const client = new LocalIpcClient(endpoint)
try {
  const response = await client.send({ GetSessionUsage: { session_id: sessionId } })
  if (!response.SessionUsage?.report) throw new Error('accounting_missing')
  process.stdout.write(JSON.stringify(response.SessionUsage.report) + '\n')
} catch { process.stderr.write('MP-08 / MP-10: kernel accounting unavailable\n'); process.exitCode = 1 }
finally { await client.close() }

// MP-08 / MP-10 / MP-11: allowlisted self-host relay status, no credential reads.
import { pathToFileURL } from 'node:url'
import path from 'node:path'
const [root, endpoint] = process.argv.slice(2)
const { LocalIpcClient } = await import(pathToFileURL(path.join(root, 'packages/kernel-client/dist/ipc.js')))
const client = new LocalIpcClient(endpoint)
try {
  const reply = await client.send({ RelayStatus: {} })
  const status = reply.RelayStatus?.status
  if (!status?.connected || typeof status.daemon_id !== 'string') throw new Error('relay not connected')
  process.stdout.write(JSON.stringify({daemon_id: status.daemon_id}) + '\n')
} catch { process.stderr.write('MP-08 / MP-10: local relay target not ready\n'); process.exitCode = 1 }
finally { await client.close() }

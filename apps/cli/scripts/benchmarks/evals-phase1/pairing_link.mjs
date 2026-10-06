// MP-08 / MP-10 / MP-11: product-generated pairing, never print runtime grants.
import { writeFile } from 'node:fs/promises'
import { pathToFileURL } from 'node:url'
import path from 'node:path'
const [root, endpoint, destination] = process.argv.slice(2)
const { LocalIpcClient } = await import(pathToFileURL(path.join(root, 'packages/kernel-client/dist/ipc.js')))
const client = new LocalIpcClient(endpoint)
try {
  const reply = await client.send({ CreateTerminalPairingLink: { terminal_type: 'cli', alias: 'Evals relay TUI', expires_in_ms: 120000 } })
  const link = reply.TerminalPairingLinkCreated?.pairing?.pairing_link
  if (typeof link !== 'string') throw new Error('pairing failed')
  await writeFile(destination, link, {mode: 0o600, flag: 'wx'})
} catch { process.stderr.write('MP-08 / MP-10: product pairing failed\n'); process.exitCode = 1 }
finally { await client.close() }

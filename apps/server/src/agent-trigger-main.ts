import { consumeKernelLocalAuthTokenFromEnv } from "@chariox/kernel-client/ipc"
import { AgentTriggerClient } from "./agent-trigger-client.js"
import { buildAgentTriggerServer, loopbackHost } from "./agent-trigger-server.js"
import { exitWhenParentPipeCloses } from "./publication-parent-pipe.js"

consumeKernelLocalAuthTokenFromEnv()
exitWhenParentPipeCloses(process.env, process.stdin, () => process.exit(0))
const session = process.env.CHARIOX_AGENT_TRIGGER_SESSION_ID
const trigger = process.env.CHARIOX_AGENT_TRIGGER_ID
const endpoint = process.env.CHARIOX_KERNEL_URL
const port = Number(process.env.PORT)
if (!session || !trigger || !endpoint || !Number.isInteger(port) || port < 1 || port > 65535) throw Error("agent-trigger requires a session, trigger, kernel and valid local port")
const host = loopbackHost(process.env.HOST ?? "127.0.0.1")
const client = new AgentTriggerClient(session, trigger, endpoint)
const app = await buildAgentTriggerServer(client, host)
await app.listen({ host, port })

await client.subscribe()

#!/usr/bin/env node
import {buildHostedAgentTriggerServer} from "./hosted-agent-trigger-server.js"
const origin = process.env.CHARIOX_AGENT_TRIGGER_PUBLIC_ORIGIN
const port = Number(process.env.PORT ?? 43110)
if (!origin || !Number.isInteger(port) || port < 1 || port > 65535) throw Error("Set CHARIOX_AGENT_TRIGGER_PUBLIC_ORIGIN and a valid PORT")
const app = buildHostedAgentTriggerServer({publicOrigin: origin, ...(process.env.CHARIOX_AGENT_TRIGGER_CONTROL_ORIGIN ? {controlOrigin: process.env.CHARIOX_AGENT_TRIGGER_CONTROL_ORIGIN} : {})})
await app.listen({host: process.env.HOST ?? "127.0.0.1", port})
for (const name of ["SIGTERM", "SIGINT"] as const) process.once(name, () => {void app.close()})

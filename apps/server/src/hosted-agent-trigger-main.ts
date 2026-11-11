#!/usr/bin/env node
import {AgentTriggerIngressClient, relayKeypairFromPrivateKey} from "@chariox/kernel-client/agent-trigger-ingress-client"
import {buildHostedAgentTriggerServer} from "./hosted-agent-trigger-server.js"
import {loadTriggerIdentity} from "./agent-trigger-identity.js"
import {pairTriggerIngress, triggerIdentityPath} from "./agent-trigger-pairing.js"
import {loadSelfHostedAdmissionConfig, remoteAdmission, selfHostedAdmission, type TriggerAdmission} from "./agent-trigger-admission.js"
// Browser session and fetch-metadata headers an admission service reads to tell who is asking.
const browserSessionHeaders = "cookie,x-chariox-browser-session,csrf-token,x-csrf-token,origin,sec-fetch-site,sec-fetch-mode,accept"
const env = process.env
const deploymentsFile = env.CHARIOX_AGENT_TRIGGER_DEPLOYMENTS_FILE
if (process.argv[2] === "pair") {
  // chariox-hosted-agent-trigger pair <terminal pairing link from the kernel>
  if (!deploymentsFile || !process.argv[3]) throw Error("usage: CHARIOX_AGENT_TRIGGER_DEPLOYMENTS_FILE=<file> pair <terminal pairing link>")
  const paired = await pairTriggerIngress(deploymentsFile, process.argv[3])
  console.log(`paired with kernel ${paired.kernelId}; relay key saved in ${paired.identityFile}`)
  process.exit(0)
}
const origin = env.CHARIOX_AGENT_TRIGGER_PUBLIC_ORIGIN
const port = Number(env.PORT ?? 43110)
if (!origin || !Number.isInteger(port) || port < 1 || port > 65535) throw Error("Set CHARIOX_AGENT_TRIGGER_PUBLIC_ORIGIN and a valid PORT")
const admissionUrl = env.CHARIOX_AGENT_TRIGGER_ADMISSION_URL
if (!admissionUrl === !deploymentsFile) throw Error("Set exactly one of CHARIOX_AGENT_TRIGGER_ADMISSION_URL (an admission service) or CHARIOX_AGENT_TRIGGER_DEPLOYMENTS_FILE (self-hosted deployments)")
const admission: TriggerAdmission = admissionUrl
  ? remoteAdmission({url: admissionUrl, publicOrigin: new URL(origin), forwardedHeaders: (env.CHARIOX_AGENT_TRIGGER_ADMISSION_HEADERS ?? browserSessionHeaders).split(",").map(name => name.trim().toLowerCase()).filter(Boolean)})
  : selfHostedAdmission(loadSelfHostedAdmissionConfig(deploymentsFile!))
// A self-hosted ingress presents the key its kernel paired; the admission service path uses a fresh key per request.
const privateKey = deploymentsFile ? loadTriggerIdentity(triggerIdentityPath(deploymentsFile)) : null
if (deploymentsFile && !privateKey) throw Error("this ingress is not paired with its kernel yet: run `pair <terminal pairing link>` first")
const keypair = privateKey ? await relayKeypairFromPrivateKey(privateKey) : undefined
const app = buildHostedAgentTriggerServer({publicOrigin: origin, admission, ...(keypair ? {connect: (kernel, bootstrap, publicOrigin) => AgentTriggerIngressClient.connect(kernel, bootstrap, publicOrigin, keypair)} : {}), ...(env.CHARIOX_AGENT_TRIGGER_SIGN_IN_PATH ? {signInPath: env.CHARIOX_AGENT_TRIGGER_SIGN_IN_PATH} : {})})
await app.listen({host: env.HOST ?? "127.0.0.1", port})
for (const name of ["SIGTERM", "SIGINT"] as const) process.once(name, () => {void app.close()})

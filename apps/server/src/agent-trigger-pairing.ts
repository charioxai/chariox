import {existsSync} from "node:fs"
import {dirname, join} from "node:path"
import {LocalIpcClient, RelayClientIdentity} from "@chariox/kernel-client/ipc"
import {joinTerminalPairingLinkRequest} from "@chariox/kernel-client/ipc-requests"
import {loadSelfHostedAdmissionConfig} from "./agent-trigger-admission.js"
import {createTriggerIdentity, loadTriggerIdentity} from "./agent-trigger-identity.js"

export const triggerIdentityPath = (deploymentsFile: string) => join(dirname(deploymentsFile), "agent-trigger-identity.json")

/**
 * The kernel admits only keys it paired. The owner creates a terminal pairing link on the kernel; this joins it with the
 * ingress's own key through the operator relay, so the self-hosted ingress needs no control plane.
 */
export async function pairTriggerIngress(deploymentsFile: string, pairingLink: string): Promise<{kernelId: string; identityFile: string}> {
  const config = loadSelfHostedAdmissionConfig(deploymentsFile)
  const payload = JSON.parse(Buffer.from(pairingLink.trim().split(".")[1] ?? "", "base64url").toString("utf8")) as {target_daemon_id?: string; terminal_id?: string}
  const kernelId = payload.target_daemon_id
  if (!kernelId) throw Error("that is not a terminal pairing link")
  if (!config.deployments.some(item => item.kernelId === kernelId)) throw Error(`no deployment in ${deploymentsFile} belongs to kernel ${kernelId}`)
  const identityFile = triggerIdentityPath(deploymentsFile)
  const key = existsSync(identityFile) ? loadTriggerIdentity(identityFile)! : createTriggerIdentity(identityFile)
  const identity = new RelayClientIdentity(key)
  const client = new LocalIpcClient(config.relay.url, {relayAuthToken: config.relay.token, targetDaemonId: kernelId, relayIdentity: identity, kernelEventStaleMs: 0})
  try {
    const response = await client.send(joinTerminalPairingLinkRequest(pairingLink, payload.terminal_id ?? null, "cli", "agent trigger ingress", identity.publicKeyThumbprint)) as Record<string, unknown>
    if (!response.TerminalPairingLinkJoined) throw Error(`the kernel did not accept the pairing link: ${JSON.stringify(response).slice(0, 200)}`)
  } finally { await client.close() }
  return {kernelId, identityFile}
}

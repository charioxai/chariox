import { setTimeout as sleep } from "node:timers/promises"

export async function waitForDaemon(LocalIpcClient, kernelUrl, listRemoteMachinesRequest) {
  let lastError = null
  for (let attempt = 0; attempt < 80; attempt += 1) {
    const client = new LocalIpcClient(kernelUrl)
    try {
      await client.send(listRemoteMachinesRequest())
      await client.close().catch(() => {})
      return
    } catch (error) {
      lastError = error
      await client.close().catch(() => {})
      await sleep(250)
    }
  }
  throw new Error(`daemon did not become ready: ${lastError?.message ?? String(lastError)}`)
}

export async function waitForRelayTarget(
  LocalIpcClient,
  relayUrl,
  relayToken,
  targetDaemonAlias,
  listRemoteMachinesRequest,
) {
  for (let attempt = 0; attempt < 80; attempt += 1) {
    const client = new LocalIpcClient(relayUrl, {
      relayAuthToken: relayToken,
      targetDaemonAlias,
      kernelPingIntervalMs: 60_000,
      kernelMaxMissedPongs: 10,
    })
    try {
      await Promise.race([
        client.send(listRemoteMachinesRequest()),
        sleep(2_000).then(() => {
          throw new Error("probe timeout")
        }),
      ])
      await client.close().catch(() => {})
      return
    } catch {
      await client.close().catch(() => {})
      await sleep(250)
    }
  }
  throw new Error(`relay target ${targetDaemonAlias} did not become reachable`)
}

export async function waitForRemoteMachine(client, machineRef, listRemoteMachinesRequest) {
  for (let attempt = 0; attempt < 80; attempt += 1) {
    const listed = unwrapVariant(await client.send(listRemoteMachinesRequest()), "RemoteMachinesListed")
    const machines = listed.machines ?? listed.remote_machines ?? []
    if (machines.some((machine) => (
      machine.machine_id === machineRef
      || machine.machine_alias === machineRef
      || machine.alias === machineRef
      || machine.display_name === machineRef
    ))) return
    await sleep(250)
  }
  throw new Error(`remote machine ${machineRef} did not become visible`)
}

function unwrapVariant(response, ...keys) {
  return keys.map((key) => response?.[key]).find((value) => value != null) ?? response
}

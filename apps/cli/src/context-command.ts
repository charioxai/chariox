// MP-05/MP-08/MP-10/MP-11: CLI projection of the shared owner-copy contract.
import { constants } from "node:fs"
import { open } from "node:fs/promises"
import {
  startOwnerManagedContextTransferRequest, getManagedContextTransferStatusRequest,
  getManagedContextLaunchTargetRequest, type OwnerManagedContextTransfer,
} from "@chariox/kernel-client/ipc-managed-context-requests"
import { LocalIpcClient } from "./ipc.js"
import { defaultKernelEndpoint, parseArgs } from "./cli-options.js"

const usage = "usage: chariox context copy SELECTION.json | status CONTEXT | launch-target CONTEXT PLAN-DIGEST [--kernel-url URL | --socket PATH]"

export async function runContextCommand(argv: string[]): Promise<boolean> {
  if (argv[0] !== "context") return false
  const args: string[] = []
  const connection: string[] = []
  for (let i = 1; i < argv.length; i++) {
    const arg = argv[i]!
    if (arg === "--kernel-url" || arg === "--socket") {
      const value = argv[++i]
      if (!value || connection.length) throw new Error(usage)
      connection.push(arg, value)
    } else args.push(arg)
  }
  if (args.length === 1 && args[0] === "--help") {
    process.stdout.write(`${usage}\nCopy requires approval in the source Project's Chariox terminal.\n`)
    return true
  }
  const options = parseArgs(connection)
  let request: unknown
  if (args[0] === "copy" && args.length === 2) {
    const file = await open(args[1]!, constants.O_RDONLY | constants.O_NONBLOCK)
    let selection: OwnerManagedContextTransfer
    try {
      if (!(await file.stat()).isFile()) throw new Error("Context selection must be a regular file")
      const bytes = Buffer.alloc(128 * 1024 + 1)
      const { bytesRead } = await file.read(bytes, 0, bytes.length, 0)
      if (bytesRead === bytes.length) throw new Error("Context selection exceeds its size limit")
      try { selection = JSON.parse(bytes.subarray(0, bytesRead).toString("utf8")) as OwnerManagedContextTransfer }
      catch { throw new Error("Invalid context selection JSON") }
    } finally { await file.close() }
    // The kernel validates selection shape, ownership and inventory; clients
    // cannot supply a ticket, source identity, credentials or plan digest.
    request = startOwnerManagedContextTransferRequest(selection)
  } else if (args[0] === "status" && args.length === 2) {
    request = getManagedContextTransferStatusRequest(args[1]!)
  } else if (args[0] === "launch-target" && args.length === 3) {
    request = getManagedContextLaunchTargetRequest(args[1]!, args[2]!)
  } else throw new Error(usage)
  const client = new LocalIpcClient(options.kernelUrl ?? options.socketPath ?? defaultKernelEndpoint())
  try {
    const response = await client.send<Record<string, unknown>>(request)
    if ("Error" in response) throw new Error(JSON.stringify(response.Error))
    process.stdout.write(`${JSON.stringify(response)}\n`)
  } finally { await client.close() }
  return true
}

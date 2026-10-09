// MP-08/MP-10/MP-11: transport facts only, outside runtime packet contents.
import type { KernelTransportDiagnostic } from "@chariox/kernel-client/ipc"
import { createProcessLogger, type CharioxLogger } from "./logging.js"

let logger: CharioxLogger | undefined
export function recordKernelTransportDiagnostic(diagnostic: KernelTransportDiagnostic): void {
  logger ??= createProcessLogger("cli", "cli.kernel_transport")
  logger.warn("kernel transport diagnostic", diagnostic)
}

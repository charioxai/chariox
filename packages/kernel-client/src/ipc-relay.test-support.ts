// Relay framing/renewal fixtures have no RTC endpoint; carrier behavior has its own real RTC tests.
import { LocalIpcClient as KernelClient } from "./ipc.js"
export class LocalIpcClient extends KernelClient {
  protected override localDirectEligible(): boolean { return false }
}

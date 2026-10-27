import type { SliceRecord } from "../cli-types.js"
import type { LocalIpcClient } from "../ipc.js"
import { listSlices } from "../slice-api.js"

export type NativeTuiSliceInventory = {
  readonly slices: readonly SliceRecord[]
  readonly error: string | null
}

export async function loadNativeTuiSliceInventory(client: LocalIpcClient): Promise<NativeTuiSliceInventory> {
  try {
    return { slices: await listSlices(client), error: null }
  } catch (error) {
    return { slices: [], error: error instanceof Error ? error.message : String(error) }
  }
}

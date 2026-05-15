import { describeCliError } from "./runtime.js"

export function isSessionUnavailableError(error: unknown): boolean {
  const message = describeCliError(error)
  return /session `[^`]+` was not found/i.test(message)
    || /attachment `[^`]+` was not found/i.test(message)
    || /does not belong to session/i.test(message)
    || /cannot perform `[^`]+` while ended/i.test(message)
}

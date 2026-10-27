import path from "node:path"
import { mkdir, writeFile } from "node:fs/promises"

export const requiredAssistantMarkers = Array.from({ length: 20 }, (_, index) => `ASSISTANT_STEP_${String(index + 1).padStart(2, "0")}`)
export const requiredToolMarkers = Array.from({ length: 20 }, (_, index) => `TOOL_STEP_${String(index + 1).padStart(2, "0")}`)
export const finalMarkerPrefix = "FINAL_EXTERNAL_PARITY_SUMMARY"

export function unwrap(response, variant) {
  if (!response || !(variant in response)) {
    throw new Error(`expected ${variant}, got ${JSON.stringify(response)}`)
  }
  return response[variant]
}

export function finalMarkerFor(marker) {
  return `${finalMarkerPrefix}_${marker}`
}

export function dedupe(values) {
  return [...new Set(values)]
}

export function normalizeLifecycleStatus(status) {
  const normalized = String(status ?? "").trim().toUpperCase()
  if (["WORKING", "RUNNING", "THINKING", "STREAMING", "BUSY"].includes(normalized)) return "WORKING"
  return normalized
}

export function countOccurrences(text, needle) {
  if (!needle) return 0
  return String(text).split(needle).length - 1
}

export function pass(name) {
  return { name, passed: true }
}

export function fail(name, details = null) {
  return { name, passed: false, details }
}

export function assertion(name, passed, details = null) {
  return { name, passed: Boolean(passed), details }
}

export async function writeJson(file, value) {
  await mkdir(path.dirname(file), { recursive: true })
  await writeFile(file, `${JSON.stringify(value, null, 2)}\n`, "utf8")
}

export function markdownCell(value) {
  return String(value ?? "")
    .replace(/\|/g, "\\|")
    .replace(/\r?\n/g, "<br>")
}

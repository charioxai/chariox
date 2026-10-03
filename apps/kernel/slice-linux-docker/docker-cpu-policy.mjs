import { readFileSync } from "node:fs"
const policy = JSON.parse(readFileSync(new URL("./docker-cpu-policy.json", import.meta.url), "utf8"))
const decimal = new RegExp(`^[0-9]+(?:\\.[0-9]{1,${policy.maximumFractionDigits}})?$`)
export function isPositiveDockerCpuLimit(value) {
  if (typeof value !== "string" || !decimal.test(value)) return false
  const [whole, fraction = ""] = value.split(".")
  const nanos = BigInt(whole) * 1_000_000_000n + BigInt(fraction.padEnd(9, "0"))
  return nanos > 0n && nanos <= BigInt(policy.maximumNanoCpus)
}

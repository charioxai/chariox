// Needle set for the "never seen" scan (CELLS.md NS-2). Pure functions; no value is ever logged by this module.
import { createHash } from "node:crypto"

export const MIN_NEEDLE_LENGTH = 16

export class NeedleRefused extends Error {}

/** Refuse needles that would match everything or nothing useful: empty, short, or one repeated character. */
export function checkNeedle(value) {
  if (typeof value !== "string" || value.length === 0) throw new NeedleRefused("needle is empty")
  if (value.length < MIN_NEEDLE_LENGTH) throw new NeedleRefused(`needle is shorter than ${MIN_NEEDLE_LENGTH} characters`)
  if (new Set(value).size === 1) throw new NeedleRefused("needle is one repeated character")
}

/** raw, base64, base64url, hex, URL-encoded, JSON-escaped, and the first/last 12 characters of the raw value. */
export function needleSet(value) {
  checkNeedle(value)
  const raw = Buffer.from(value, "utf8")
  const forms = new Map()
  const add = (name, text) => { if (typeof text === "string" && text.length >= 12 && ![...forms.values()].some(b => b.equals(Buffer.from(text, "utf8")))) forms.set(name, Buffer.from(text, "utf8")) }
  add("raw", value)
  add("base64", raw.toString("base64"))
  add("base64url", raw.toString("base64url"))
  // The value inside a larger base64 blob sits at byte offset 1 or 2 of a 3-byte group: keep only the characters that do not depend on the neighbours.
  for (const [shift, skip] of [[1, 2], [2, 3]]) add(`base64@${shift}`, Buffer.concat([Buffer.alloc(shift), raw]).toString("base64").slice(skip, -4))
  add("hex", raw.toString("hex"))
  add("url", encodeURIComponent(value))
  add("json", JSON.stringify(value).slice(1, -1))
  add("first12", value.slice(0, 12))
  add("last12", value.slice(-12))
  return [...forms].map(([name, bytes]) => ({ name, bytes }))
}

/** Short hash so two runs can be compared without the value. */
export const needleHash = value => createHash("sha256").update(value).digest("hex").slice(0, 8)

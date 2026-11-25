/** `--dpr N`: absent means 1; a present value must be 1 or 2 (the mirror accepts only those). */
export function dprFrom(argv) {
  const at = argv.indexOf("--dpr")
  if (at < 0) return 1
  const value = Number(argv[at + 1])
  if (value !== 1 && value !== 2) throw new Error(`--dpr must be 1 or 2, got ${argv[at + 1] ?? "nothing"}`)
  return value
}

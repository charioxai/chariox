// Per-surface scan with LIVE decoys (Mac 11-25): one pass over every scanned surface for several needle groups at once.
// A group is a decoy (a known value the drill planted through the real path of one surface) or a real secret. The result is a matrix
// group x surface of hit counts; a surface "can see" only when its own live decoy is found there, and a real secret is clean only when
// it has zero hits on a surface that can see. Counts, forms and offsets only: no value is returned or printed.
import { needleSet } from "./needles.mjs"
import { scanRoots } from "./scan.mjs"

/** surfaces: [{label, kind: "dir"|"proc", path}]; groups: [{label, value, role: "decoy"|"real", surface?}]. */
export async function surfaceMatrix({ surfaces, groups, ocr = false, reader }) {
  const needles = groups.flatMap(g => needleSet(g.value).map(n => ({ ...n, group: g.label })))
  const roots = surfaces.map(s => ({ target: s.label, kind: s.kind ?? "dir", path: s.path }))
  const { roots: scanned, ocrAvailable } = await scanRoots(roots, needles, { ocr, ...(reader ? { reader } : {}) })
  const matrix = {}, cell = (group, surface) => (matrix[group] ??= {})[surface] ??= { bytes: 0, ocr: 0, files: new Set() }
  for (const root of scanned) for (const hit of root.hits) {
    // /proc hits split into argv and environ so each is its own surface.
    const surface = root.target === "process" ? (hit.path.endsWith("environ") ? "process-environ" : "process-argv") : root.target
    const c = cell(hit.group, surface)
    if (hit.via === "ocr") c.ocr++; else c.bytes++
    c.files.add(hit.path)
  }
  const out = {}
  for (const [group, bySurface] of Object.entries(matrix)) out[group] = Object.fromEntries(Object.entries(bySurface).map(([s, c]) => [s, { bytes: c.bytes, ocr: c.ocr, files: c.files.size }]))
  return {
    ocrAvailable,
    roots: scanned.map(r => ({ surface: r.target, path: r.path, files: r.files, bytes: r.bytes, images: r.images, ocr_ok: r.ocr_ok, not_scanned: r.not_scanned, unreadable: r.unreadable, unreadable_paths: r.unreadable_paths })),
    matrix: out,
    hitFiles: Object.fromEntries(Object.entries(matrix).map(([g, by]) => [g, Object.fromEntries(Object.entries(by).map(([s, c]) => [s, [...c.files].slice(0, 6)]))])),
  }
}

/**
 * Verdict per surface, in this order:
 *  1. FAIL when any real secret has a hit on the surface (a positive match is a failure whatever the coverage),
 *  2. BLOCKED when an expected real needle is unavailable (the claim "0 hits for every real secret" cannot be made),
 *  3. BLOCKED when the surface's evidence is incomplete (unreadable files, images OCR could not read; `coverage[surface]`),
 *  4. BLOCKED when its own live decoy was not found there,
 *  5. PASS.
 * decoyOf maps a surface to the decoy group planted for it; expectedReal lists the real group labels that must all have been scanned.
 */
export function surfaceVerdicts({ surfaces, matrix, groups, decoyOf, requireOcrFor = [], expectedReal = null, coverage = {} }) {
  const hits = (group, surface, ocrOnly) => { const c = matrix[group]?.[surface]; return c ? (ocrOnly ? c.ocr : c.bytes + c.ocr) : 0 }
  const realGroups = groups.filter(g => g.role === "real")
  const missing = (expectedReal ?? realGroups.map(g => g.label)).filter(label => !realGroups.some(g => g.label === label))
  return surfaces.map(label => {
    const decoy = decoyOf[label]
    const leaks = realGroups.map(g => ({ group: g.label, hits: hits(g.label, label) })).filter(x => x.hits)
    const seen = decoy ? hits(decoy, label, requireOcrFor.includes(label)) : 0
    if (leaks.length) return { surface: label, verdict: "FAIL", decoy, decoy_hits: seen, leaks }
    if (missing.length) return { surface: label, verdict: "BLOCKED", decoy, reason: `no needle for ${missing.join(", ")}: a clean result would not cover every real secret` }
    const cov = coverage[label]
    if (cov?.unreadable || cov?.not_scanned) return { surface: label, verdict: "BLOCKED", decoy, reason: `incomplete evidence: ${cov.unreadable ?? 0} unreadable file(s), ${cov.not_scanned ?? 0} image(s) OCR could not read` }
    if (!decoy) return { surface: label, verdict: "BLOCKED", reason: "no live decoy path for this surface" }
    if (!seen) return { surface: label, verdict: "BLOCKED", decoy, reason: `decoy ${decoy} not found${requireOcrFor.includes(label) ? " by OCR" : ""}: it never reached this surface or the scan missed it` }
    return { surface: label, verdict: "PASS", decoy, decoy_hits: seen }
  })
}

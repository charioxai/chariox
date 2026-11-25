// Per-surface scan with LIVE decoys (Mac 11-25): one pass over every scanned surface for several needle groups at once.
// A group is a decoy (a known value the drill planted through the real path of one surface) or a real secret. The result is a matrix
// group x surface of hit counts; a surface "can see" only when its own live decoy is found there, and a real secret is clean only when
// it has zero hits on a surface that can see. Counts, forms and offsets only: no value is returned or printed.
import { needleSet } from "./needles.mjs"
import { scanRoots } from "./scan.mjs"

/** surfaces: [{label, kind: "dir"|"proc", path}]; groups: [{label, value, role: "decoy"|"real", surface?}]. */
export async function surfaceMatrix({ surfaces, groups, ocr = false }) {
  const needles = groups.flatMap(g => needleSet(g.value).map(n => ({ ...n, group: g.label })))
  const roots = surfaces.map(s => ({ target: s.label, kind: s.kind ?? "dir", path: s.path }))
  const { roots: scanned, ocrAvailable } = await scanRoots(roots, needles, { ocr })
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
    roots: scanned.map(r => ({ surface: r.target, path: r.path, files: r.files, bytes: r.bytes, images: r.images, ocr_ok: r.ocr_ok, not_scanned: r.not_scanned })),
    matrix: out,
    hitFiles: Object.fromEntries(Object.entries(matrix).map(([g, by]) => [g, Object.fromEntries(Object.entries(by).map(([s, c]) => [s, [...c.files].slice(0, 6)]))])),
  }
}

/**
 * Verdict per surface. decoyOf maps a surface to the decoy group planted for it. A surface is:
 *  - BLOCKED when its decoy was not found (the scan cannot be trusted there: either the decoy never reached it or the scan missed it),
 *  - FAIL when a real secret has a hit on it, PASS when the decoy is found there and every real secret has zero hits.
 */
export function surfaceVerdicts({ surfaces, matrix, groups, decoyOf, requireOcrFor = [] }) {
  const hits = (group, surface, ocrOnly) => { const c = matrix[group]?.[surface]; return c ? (ocrOnly ? c.ocr : c.bytes + c.ocr) : 0 }
  return surfaces.map(label => {
    const decoy = decoyOf[label]
    if (!decoy) return { surface: label, verdict: "BLOCKED", reason: "no live decoy path for this surface" }
    const seen = hits(decoy, label, requireOcrFor.includes(label))
    if (!seen) return { surface: label, verdict: "BLOCKED", decoy, reason: `decoy ${decoy} not found${requireOcrFor.includes(label) ? " by OCR" : ""}: it never reached this surface or the scan missed it` }
    const leaks = groups.filter(g => g.role === "real").map(g => ({ group: g.label, hits: hits(g.label, label) })).filter(x => x.hits)
    return leaks.length ? { surface: label, verdict: "FAIL", decoy, decoy_hits: seen, leaks } : { surface: label, verdict: "PASS", decoy, decoy_hits: seen }
  })
}

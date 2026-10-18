// MP-08 / MP-10 / MP-11: interpret non-secret, read-only drill observations.
export function summarizeBriefTransition(before, after) {
  const previous = before.brief?.coveredThroughSequence ?? 0
  const current = after.brief?.coveredThroughSequence ?? 0
  return {
    previousWatermark: previous,
    currentWatermark: current,
    watermarkRegressed: current < previous,
    briefChanged: before.brief?.sha256 !== after.brief?.sha256,
    sections: after.brief?.sections ?? [],
    hasStoredBrief: Boolean(after.brief),
  }
}

export function foreignNativeSession(source, target) {
  if (!source.nativeId || !target.nativeId) return { verified: false, reused: null }
  return { verified: true, reused: source.provider !== target.provider && source.nativeId === target.nativeId }
}

export function formatContextValidationFailure(error) {
  return String(error).split('\n')[0]
    .replace(/(?:__Host-)?chariox_[a-z_]+=[^;\s]+/gi, '[redacted validation cookie]')
    .replace(/Bearer\s+[^\s]+/gi, '[redacted authorization]')
}

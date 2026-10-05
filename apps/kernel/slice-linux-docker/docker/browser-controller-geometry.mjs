// MP-08/MP-10/MP-11: observed CSS geometry/appearance, never inline CSS/source.
export const SNAPSHOT_COMPUTED_STYLES = Object.freeze(['visibility','opacity','display',
  'background-color','border-top-color','border-top-width','border-top-style','color']);

export function computedAppearanceByNode(layout, strings, maxStringLength) {
  const result = new Map();
  for (let i=0;i<(layout?.nodeIndex?.length ?? 0);i++) {
    const appearance = {};
    for (let j=3;j<SNAPSHOT_COMPUTED_STYLES.length;j++) {
      const value = strings[layout.styles?.[i]?.[j]];
      if (typeof value === 'string' && Buffer.byteLength(value) <= maxStringLength)
        appearance[`chariox-rendered-${SNAPSHOT_COMPUTED_STYLES[j]}`] = value;
    }
    result.set(layout.nodeIndex[i],appearance);
  }
  return result;
}

export function withComputedAppearance(attributes, appearance = {}, maxAttributes) {
  // Page-authored attributes cannot impersonate computed visual observations.
  const computed = Object.entries(appearance).slice(0,maxAttributes);
  const authored = Object.entries(attributes).filter(([name]) => !name.startsWith('chariox-rendered-'))
    .slice(0,Math.max(0,maxAttributes-computed.length));
  return Object.fromEntries([...authored,...computed]);
}

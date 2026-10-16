// MP-08/MP-10: damage-driven native-control tiles. A control's pixels follow its own record, its private UA
// state (hover, focus, value, label) and the records drawn over or under it. A read where none of those
// changed reuses the previous capture instead of screenshotting the control again.
const paint = node => JSON.stringify({ ...node, children: undefined, box: undefined });
const overlaps = (a, b) => a.x < b.x + b.width && b.x < a.x + a.width && a.y < b.y + b.height && b.y < a.y + a.height;

// Viewport box of a record, offset through its frame ancestors.
export function globalBoxes(byId) {
  return node => {
    const box = { ...node.box };
    for (let ancestor = byId.get(node.parent); ancestor; ancestor = byId.get(ancestor.parent)) if (ancestor.kind === 'frame') {
      box.x += ancestor.box.x + (parseFloat(ancestor.style?.['border-left-width']) || 0) + (parseFloat(ancestor.style?.['padding-left']) || 0);
      box.y += ancestor.box.y + (parseFloat(ancestor.style?.['border-top-width']) || 0) + (parseFloat(ancestor.style?.['padding-top']) || 0);
    }
    return box;
  };
}

// Identity of a native control's pixels; undefined (never reused) without the observer's private state.
export function tileKeys(tiles, boxes, states) {
  const keys = new Map();
  for (const tile of tiles) if (tile.reason === 'native_control' && typeof states?.[tile.id] === 'string') keys.set(tile.id, JSON.stringify([tile, boxes.get(tile.id), states[tile.id]]));
  return keys;
}

// Records that changed, appeared or vanished between two reads, at the position they had and have.
export function damageOf(changed, removed, previous, boxOf, previousBoxOf) {
  const damage = [];
  for (const node of changed) {
    const was = previous.get(node.id);
    if (node.box || was?.box) damage.push({ id: node.id, now: node.box && boxOf(node), was: was?.box && previousBoxOf(was), container: Boolean(was) && paint(node) === paint(was) });
  }
  for (const id of removed) { const was = previous.get(id); if (was?.box) damage.push({ id, was: previousBoxOf(was) }); }
  return damage;
}

export class TileCache {
  constructor() { this.entries = new Map(); this.scale = 0; this.clean = false; }
  // Tiles to capture again. An ancestor whose own paint did not change (it only grew or gained children)
  // leaves the pixels of the control inside it alone.
  stale(tiles, keys, boxes, scale, damage, byId) {
    if (!this.clean || this.scale !== scale) return tiles;
    const inside = (ancestor, id) => { for (let n = byId.get(id)?.parent; n; n = byId.get(n)?.parent) if (n === ancestor) return true; return false; };
    return tiles.filter(tile => {
      const key = keys.get(tile.id);
      if (key === undefined || this.entries.get(tile.id)?.key !== key) return true;
      const box = boxes.get(tile.id);
      return damage.some(d => d.id !== tile.id && !(d.container && inside(d.id, tile.id)) && [d.now, d.was].some(rect => rect && overlaps(rect, box)));
    });
  }
  // `captured` maps the tiles screenshotted this read to their packet entries. Tiles that were not captured
  // keep their entry. A capture under a protected region is never reused (the region can move under a tile).
  store(tiles, keys, captured, scale, protectedRegions) {
    const entries = new Map();
    for (const { id } of tiles) {
      const entry = captured.has(id) ? { key: keys.get(id), data: captured.get(id) } : this.entries.get(id);
      if (entry) entries.set(id, entry);
    }
    this.entries = entries; this.scale = scale; this.clean = protectedRegions === 0;
  }
  data(id) { return this.entries.get(id)?.data; }
}

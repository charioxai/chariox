// MD-DISPLAY-02: exact RGB dirty tiles for settled-screen refinement.
export function dirtyTiles(reference, previous, PNG, size = 128) {
  const image = PNG.sync.read(reference), old = previous && PNG.sync.read(previous);
  const tiles = [];
  for (let y = 0; y < image.height; y += size) for (let x = 0; x < image.width; x += size) {
    const width = Math.min(size, image.width - x), height = Math.min(size, image.height - y);
    let changed = !old || old.width !== image.width || old.height !== image.height;
    for (let j = y; !changed && j < y + height; j++) for (let i = x; !changed && i < x + width; i++) {
      const offset = (j * image.width + i) * 4;
      changed = image.data.subarray(offset, offset + 3).compare(old.data.subarray(offset, offset + 3)) !== 0;
    }
    if (changed) {
      const tile = new PNG({width, height}); PNG.bitblt(image, tile, x, y, width, height, 0, 0);
      tiles.push({x, y, width, height, bytes: PNG.sync.write(tile)});
    }
  }
  return tiles;
}

// Screenshot-region masks from trusted CDP metadata, never page JavaScript.
export async function protectedHostRegions(connection, sessionId) {
  const { root } = await connection.send("DOM.getDocument", { depth: -1, pierce: true }, sessionId);
  const { nodeIds } = await connection.send("DOM.querySelectorAll", {
    nodeId: root.nodeId,
    selector: 'input[type="password"], [data-chariox-observation-protected], input[autocomplete="one-time-code"], input[autocomplete="cc-number"], input[autocomplete="cc-csc"], iframe, frame',
  }, sessionId);
  if (!Array.isArray(nodeIds)) throw new Error("Capture protection unavailable");
  const nodes = [...nodeIds], pending = [root];
  let visited = 0;
  while (pending.length) {
    if (++visited > 100_000) throw new Error("Capture protection tree limit exceeded");
    const node = pending.pop();
    if (node.shadowRoots?.length) nodes.push(node.nodeId);
    pending.push(...(node.children ?? []));
  }
  if (nodes.length > 1024) throw new Error("Capture protection limit exceeded");
  const result = [];
  for (const nodeId of new Set(nodes)) {
    const { model } = await connection.send("DOM.getBoxModel", { nodeId }, sessionId);
    const quad = model?.border;
    if (!Array.isArray(quad) || quad.length !== 8 || quad.some(n => !Number.isFinite(n))) {
      throw new Error("Capture protection bounds unavailable");
    }
    const xs = [quad[0], quad[2], quad[4], quad[6]], ys = [quad[1], quad[3], quad[5], quad[7]];
    result.push({ x: Math.min(...xs), y: Math.min(...ys), width: Math.max(...xs) - Math.min(...xs), height: Math.max(...ys) - Math.min(...ys) });
  }
  return result;
}

export async function captureRegionMasks(connection, sessionId) {
  // Layout changes or failed metadata checks cannot reveal an unmapped field.
  const before = await protectedHostRegions(connection, sessionId);
  return { async afterCapture({ width = 1280, height = 800 } = {}) {
    if (!Number.isFinite(width) || !Number.isFinite(height) || width <= 0 || height <= 0) throw new Error("Capture geometry unavailable");
    const fullFrame = [{ x: 0, y: 0, width, height }];
    try {
      const after = await protectedHostRegions(connection, sessionId);
      if (JSON.stringify(before) !== JSON.stringify(after)) return fullFrame;
      // CDP bounds are CSS coordinates; the raster crop masks native PNG pixels.
      return after.map(region => ({ x: region.x * width / 1280, y: region.y * height / 800,
        width: region.width * width / 1280, height: region.height * height / 800 }));
    } catch { return fullFrame; }
  } };
}

import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { readFileSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { decodePng } from '../kernel/slice-linux-docker/docker/kernel-browser-pixels.mjs';

export function verifyNativeCapture(before, after, scale, evidence) {
  const binary = '/Users/miguel/.chariox/dev/cumac/build/r7-tests/capture-replay';
  const packets = JSON.parse(execFileSync(binary, [before, after, String(scale)], { maxBuffer: 3 * 1024 * 1024 }));
  const [base, damage, repair] = packets;
  assert.equal(base.kind, 'png'); assert.equal(repair.kind, 'png');
  assert.equal(repair.base_sequence, undefined);
  const initial = decodePng(base.data_base64, scale);
  const expected = decodePng(readFileSync(after).toString('base64'), scale);
  let replay = { ...initial, pixels: Buffer.from(initial.pixels) };
  if (damage.kind === 'tiles') {
    assert.equal(damage.base_sequence, base.sequence);
    for (const tile of damage.tiles) {
      const patch = decodePng(tile.data_base64, scale);
      assert.equal(patch.width, tile.width); assert.equal(patch.height, tile.height);
      for (let row = 0; row < tile.height; row++) patch.pixels.copy(replay.pixels,
        ((tile.y + row) * replay.width + tile.x) * 4, row * tile.width * 4, (row + 1) * tile.width * 4);
    }
  } else { assert.equal(damage.kind, 'png'); replay = decodePng(damage.data_base64, scale); }
  assert.ok(replay.pixels.equals(expected.pixels), 'native damage reconstruction differs from independent PNG decode');
  assert.ok(decodePng(repair.data_base64, scale).pixels.equals(expected.pixels), 'exact repair differs from capture');
  assert.ok(!initial.pixels.equals(expected.pixels), 'fixture input must create visible damage');
  writeFileSync(path.join(evidence, 'native-capture-packets.json'), JSON.stringify(packets), { mode: 0o600 });
  return { name: 'live-protected-capture-native-damage-replay', passed: true,
    damage_kind: damage.kind, tile_count: damage.tiles?.length ?? 0, width: expected.width, height: expected.height,
    scope: 'paired-r6-capture-and-r7-private-attachment-replay', encrypted_viewer_delivery: false };
}

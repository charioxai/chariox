// MP-08/MP-10/MP-11: stale artifact and shared-builder admission negatives.
import assert from "node:assert/strict";
import test from "node:test";
import { assertConcurrencyArtifact, assertConcurrencyResources, concurrencyTestSymbol } from "./browser-controller-concurrency-preflight.mjs";

test("MP-08/MP-10/MP-11 rejects a replaced binary even with the right test symbol", () => {
  const input = { sha256: "a".repeat(64), expectedSha256: "a".repeat(64), listing: `${concurrencyTestSymbol}: test\n\n1 test, 0 benchmarks\n` };
  assert.doesNotThrow(() => assertConcurrencyArtifact(input));
  assert.throws(() => assertConcurrencyArtifact({ ...input, sha256: "b".repeat(64) }), /digest changed/);
  assert.throws(() => assertConcurrencyArtifact({ ...input, expectedSha256: "invalid" }), /invalid expected/);
});

test("MP-08/MP-10/MP-11 rejects zero tests, another symbol and duplicate symbols", () => {
  const sha256 = "a".repeat(64);
  for (const listing of ["0 tests, 0 benchmarks", "other::acceptance: test", `${concurrencyTestSymbol}: test\n${concurrencyTestSymbol}: test`]) {
    assert.throws(() => assertConcurrencyArtifact({ sha256, listing }), /missing or ambiguous/);
  }
});

test("MP-08/MP-10/MP-11 enforces the assigned 16 GiB memory floor and disk headroom", () => {
  const sample = { diskBytes: 10 * 1024 ** 3, memAvailableKiB: 16 * 1024 ** 2 };
  assert.doesNotThrow(() => assertConcurrencyResources(sample, 16));
  assert.throws(() => assertConcurrencyResources({ ...sample, memAvailableKiB: sample.memAvailableKiB - 1 }, 16), /resource floor/);
  assert.throws(() => assertConcurrencyResources({ ...sample, diskBytes: sample.diskBytes - 1 }, 16), /resource floor/);
  for (const invalid of [0, -1, NaN, 1.5]) assert.throws(() => assertConcurrencyResources(sample, invalid), /positive integer/);
});

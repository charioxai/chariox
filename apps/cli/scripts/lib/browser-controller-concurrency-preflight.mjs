// MP-08/MP-10/MP-11: fail before provisioning when the selected artifact or
// builder resource floor does not match the lane's retained receipt.
import assert from "node:assert/strict";

export const concurrencyTestSymbol = "runtime::state::tool_dispatch::slice::controller_browser::concurrency_drill::headed_controller_concurrency_acceptance";
// Debug-build kernel futures exceed Linux's default 2 MiB test-thread stack.
export const concurrencyRustMinStackBytes = 32 * 1024 * 1024;

export function assertConcurrencyArtifact({ sha256, expectedSha256, listing }) {
  if (expectedSha256 !== undefined) {
    assert.match(expectedSha256, /^[a-f0-9]{64}$/, "MP-08/MP-10/MP-11 invalid expected artifact SHA-256");
    assert.equal(sha256, expectedSha256, "MP-08/MP-10/MP-11 test artifact digest changed");
  }
  const symbols = listing.split("\n").map(line => line.trim()).filter(line => line.endsWith(": test"));
  assert.deepEqual(symbols, [concurrencyTestSymbol + ": test"],
    "MP-08/MP-10/MP-11 exact ignored acceptance test is missing or ambiguous");
}

export function assertConcurrencyResources(sample, memoryFloorGiB = 4) {
  assert.ok(Number.isSafeInteger(memoryFloorGiB) && memoryFloorGiB > 0,
    "MP-08/MP-10/MP-11 memory floor must be a positive integer GiB value");
  assert.ok(sample.diskBytes >= 10 * 1024 ** 3 && sample.memAvailableKiB >= memoryFloorGiB * 1024 ** 2,
    "MP-08/MP-10/MP-11 resource floor reached");
}

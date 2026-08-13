import assert from "node:assert/strict"
import path from "node:path"
import test from "node:test"

import {
  rustBinaryPath,
  rustManifestPath,
} from "./rust-workspace.mjs"

const repoRoot = path.resolve("/tmp/chariox-rust-workspace-test")

test("Rust binaries use the Cargo workspace target directory", () => {
  assert.equal(
    rustBinaryPath(repoRoot, "chariox-kernel", {}),
    path.join(repoRoot, "target/debug/chariox-kernel"),
  )
  assert.equal(
    rustBinaryPath(repoRoot, "chariox-relay", {}),
    path.join(repoRoot, "target/debug/chariox-relay"),
  )
})

test("Rust binary paths honor CARGO_TARGET_DIR", () => {
  assert.equal(
    rustBinaryPath(repoRoot, "chariox-cli", { CARGO_TARGET_DIR: ".cache/cargo-target" }),
    path.join(repoRoot, ".cache/cargo-target/debug/chariox-cli"),
  )
  assert.equal(
    rustBinaryPath(repoRoot, "chariox-kernel", { CARGO_TARGET_DIR: "/tmp/custom-target" }),
    "/tmp/custom-target/debug/chariox-kernel",
  )
})

test("Rust binary manifests follow workspace package ownership", () => {
  assert.equal(rustManifestPath(repoRoot, "chariox-cli"), path.join(repoRoot, "apps/kernel/Cargo.toml"))
  assert.equal(rustManifestPath(repoRoot, "chariox-kernel"), path.join(repoRoot, "apps/kernel/Cargo.toml"))
  assert.equal(rustManifestPath(repoRoot, "chariox-relay"), path.join(repoRoot, "apps/relay/Cargo.toml"))
  assert.throws(() => rustBinaryPath(repoRoot, "unknown", {}), /unsupported Rust binary unknown/)
})

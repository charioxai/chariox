import assert from "node:assert/strict";
import test from "node:test";
import { BrowserFrameTargets } from "./browser-controller-frames.mjs";

const tree = (id, parentId, childFrames = []) => ({ frame: { id, parentId }, childFrames });

test("frame ownership covers isolated trees and removes descendants without losing other tabs", () => {
  const registry = new BrowserFrameTargets();
  registry.replace("a", [tree("root-a", undefined, [tree("child-a", "root-a")]), tree("isolated-a", "child-a", [tree("grandchild-a", "isolated-a")])]);
  registry.replace("b", [tree("root-b")]);
  for (const id of ["root-a", "child-a", "isolated-a", "grandchild-a"]) assert.equal(registry.get(id), "a");
  registry.record({ method: "Page.frameDetached", params: { frameId: "child-a", reason: "swap" } }, "a");
  assert.equal(registry.get("grandchild-a"), "a", "renderer swaps preserve logical frame ownership");
  registry.record({ method: "Page.frameDetached", params: { frameId: "child-a", reason: "remove" } }, "a");
  for (const id of ["child-a", "isolated-a", "grandchild-a"]) assert.equal(registry.get(id), undefined);
  assert.equal(registry.get("root-a"), "a");
  assert.equal(registry.get("root-b"), "b");
  registry.removeTarget("a");
  assert.equal(registry.get("root-a"), undefined);
  assert.equal(registry.get("root-b"), "b");
  registry.clear();
  assert.equal(registry.get("root-b"), undefined);
});

test("late frame events update ownership while root navigation replaces the old tree", () => {
  const registry = new BrowserFrameTargets();
  registry.replace("a", [tree("root-a")]);
  registry.record({ method: "Page.frameAttached", params: { frameId: "late", parentFrameId: "root-a" } }, "a");
  registry.record({ method: "Page.frameNavigated", params: { frame: { id: "late", parentId: "root-a" } } }, "a");
  assert.equal(registry.get("late"), "a");
  registry.record({ method: "Page.frameNavigated", params: { frame: { id: "root-a" } } }, "a");
  assert.equal(registry.get("late"), undefined);
  assert.equal(registry.get("root-a"), "a");
});

test("frame retention has a per-tab bound and releases capacity on removal", () => {
  const registry = new BrowserFrameTargets();
  registry.replace("a", [tree("root-a")]);
  for (let i = 0; i < 1_000; i++) registry.record({ method: "Page.frameAttached", params: { frameId: `child-${i}`, parentFrameId: "root-a" } }, "a");
  assert.equal(registry.get("child-62"), "a");
  assert.equal(registry.get("child-63"), undefined);
  registry.record({ method: "Page.frameDetached", params: { frameId: "child-0", reason: "remove" } }, "a");
  registry.record({ method: "Page.frameAttached", params: { frameId: "new", parentFrameId: "root-a" } }, "a");
  assert.equal(registry.get("new"), "a");
  registry.replace("b", [tree("root-b")]);
  assert.equal(registry.get("root-b"), "b");
});

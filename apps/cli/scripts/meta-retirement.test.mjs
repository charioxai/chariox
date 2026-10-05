// MP-08 / MP-11: retirement is one shared kernel command catalog.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const core = JSON.parse(readFileSync(new URL("../../kernel/src/runtime/terminal_command_catalog/catalog/core.json", import.meta.url)));
function flatten(nodes) { return nodes.flatMap(node => [node, ...flatten(node.children ?? [])]); }
test("retired Meta command and task controls are absent; sudo remains", () => {
  const nodes = flatten(core);
  assert.equal(nodes.some(node => node.id === "meta" || node.id === "agent-task"), false);
  assert.equal(nodes.some(node => node.value === "/sudo "), true);
});

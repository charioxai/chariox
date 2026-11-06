import assert from "node:assert/strict";
import test from "node:test";
import { sanitizeMirrorTree } from "./kernel-browser-mirror-sanitize.mjs";

const box = { x: 0, y: 0, width: 100, height: 20 };
const tree = () => ({
  fonts: [],
  nodes: [
    { id: "prose", kind: "element", parent: null, box, attributes: {} },
    { id: "prose-text", kind: "text", parent: "prose", box, text: "Your token is ready" },
    { id: "link", kind: "element", parent: null, box, attributes: { href: "https://x.test/keys#key_token_1" } },
  ],
});

test("a short protected value hides prose but leaves a link whose URL contains the word as a DOM node", () => {
  const source = tree();
  assert.equal(sanitizeMirrorTree(source, ["token"]), false);
  const [prose, , link] = source.nodes;
  assert.equal(prose.kind, "tile", "the sentence holds the word as a whole word");
  assert.equal(link.kind, "element");
  assert.equal(link.attributes.href, "https://x.test/keys#key_token_1");
});

test("a long protected value in a URL still turns its link into a tile", () => {
  const source = tree();
  source.nodes[2].attributes.href = "https://x.test/keys#key_vaudit-long-secret-value-12345";
  assert.equal(sanitizeMirrorTree(source, ["vaudit-long-secret-value-12345"]), false);
  assert.equal(source.nodes[2].kind, "tile");
});

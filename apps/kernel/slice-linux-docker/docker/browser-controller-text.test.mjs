import assert from "node:assert/strict";
import test from "node:test";
import { browserTextPage, MAX_TEXT_PAGE_BYTES, renderedSnapshotText } from "./browser-controller-text.mjs";

// MP-08 / MP-10 / MP-11: bounded JSON and UTF-8 continuation, no fixture gold.
test("rendered text pages preserve UTF-8 exactly and keep escaped JSON bounded", () => {
  const text = '😀 日本 "\\\n'.repeat(1000);
  let offset = 0;
  let rebuilt = "";
  do {
    const page = browserTextPage(text, { offset, max_bytes: 257 });
    assert.ok(Buffer.byteLength(page.text) <= 257);
    assert.deepEqual(JSON.parse(JSON.stringify(page)), page);
    rebuilt += page.text;
    offset = page.next_offset;
  } while (offset !== null);
  assert.equal(rebuilt, text);
  const page = browserTextPage('\u0001'.repeat(10000));
  assert.ok(Buffer.byteLength(JSON.stringify({ text: page.text, ...page })) < 16 * 1024);
  assert.equal(Buffer.byteLength(page.text), MAX_TEXT_PAGE_BYTES);
});

test("text query retains adjacent row context and explicit empty no-match", () => {
  assert.equal(browserTextPage("header\nrow 1\t7\nrow 2\t7\nfooter", { query: "row 2" }).text, "row 1\t7\nrow 2\t7\nfooter");
  assert.deepEqual(browserTextPage("content", { query: "absent" }), { text: "", offset: 0, next_offset: null, total_bytes: 0, query: "absent" });
});

test("invalid text bounds and split-codepoint offsets fail before projection", () => {
  for (const request of [{ offset: -1 }, { offset: 1.5 }, { max_bytes: 0 }, { max_bytes: 1025 }, { query: "a".repeat(2049) }, { offset: 1 }]) {
    assert.throws(() => browserTextPage("😀", request), { code: "browser_text_invalid" });
  }
});

// MP-08 / MP-10 / MP-11: exact-copy boundaries come from rendered text.
function textSnapshot(content, tags = []) {
  const strings = ["PRE", "#text", content, "BR", "after"];
  return { strings, documents: [{
    nodes: { nodeType: [1, 3, ...tags.map(() => 1), 3], nodeName: [0, 1, ...tags.map(() => 3), 1], parentIndex: [-1, 0, ...tags.map(() => 0), 0] },
    layout: { nodeIndex: [0, 1, ...tags.map((_, i) => i + 2), tags.length + 2], text: [-1, 2, ...tags.map(() => -1), tags.length ? 4 : -1], styles: [] },
  }] };
}

test("MP-08 P3 rendered capture preserves literal leading and trailing newlines", () => {
  const expected = "\n  indented\n\n";
  assert.equal(renderedSnapshotText(textSnapshot(expected)), expected);
});

test("MP-08 P3 rendered BR nodes preserve line and blank-line boundaries", () => {
  assert.equal(renderedSnapshotText(textSnapshot("before", ["BR", "BR"])), "before\n\nafter");
});

// MP-08 / MP-10 / MP-11: visibility can be overridden; opacity/display cannot.
test("MP-08 P3 visible descendants override ancestor visibility only", () => {
  for (const [parentVisibility, opacity, display, childVisibility, expected] of [
    ["hidden", "1", "block", "visible", "Visible child"],
    ["collapse", "1", "block", "visible", "Visible child"],
    ["visible", "0", "block", "visible", ""],
    ["visible", "1", "none", "visible", ""],
    ["visible", "1", "block", "hidden", ""],
  ]) {
    const strings = ["DIV", "#text", "Visible child", parentVisibility, opacity, display, childVisibility, "1", "block"];
    const snapshot = { strings, documents: [{
      nodes: { nodeType: [1, 3], nodeName: [0, 1], parentIndex: [-1, 0] },
      layout: { nodeIndex: [0, 1], text: [-1, 2], styles: [[3, 4, 5], [6, 7, 8]] },
    }] };
    assert.equal(renderedSnapshotText(snapshot), expected);
  }
});

// MP-08 / MP-10 / MP-11: empty frames add no invented exact-copy bytes.
test("MP-08 P3 empty documents add no separators to rendered text", () => {
  const populated = textSnapshot("\n  authored\n\n");
  const empty = { nodes: { nodeType: [], nodeName: [], parentIndex: [] }, layout: { nodeIndex: [], text: [] } };
  populated.documents = [empty, ...populated.documents, empty];
  assert.equal(renderedSnapshotText(populated), "\n  authored\n\n");
  assert.equal(renderedSnapshotText({ strings: [], documents: [empty, empty] }), "");
});

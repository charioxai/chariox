import test from "node:test";
import assert from "node:assert/strict";
import { VisitedLinks } from "./kernel-browser-visited-links.mjs";

const requested = (url, disposition = "currentTab") => ({ method: "Page.frameRequestedNavigation", params: { frameId: "T", url, disposition } });
const committed = (url, urlFragment) => ({ method: "Page.frameNavigated", params: { frame: { id: "T", url, ...(urlFragment ? { urlFragment } : {}) } } });

test("MP-08/MP-10 protocol 593: a visit counts on the origin that followed the link, a typed URL on its own", () => {
  const visited = new VisitedLinks(), seen = [];
  visited.listeners.add((origin, url) => seen.push(`${origin} ${url}`));
  visited.observe("T", requested("https://b.test/b"), "https://a.test/");
  visited.observe("T", committed("https://b.test/b/", "#top"), "https://a.test/");
  assert.deepEqual(visited.from("https://a.test"), ["https://b.test/b", "https://b.test/b/#top"], "a redirected visit marks the asked and the committed URL");
  assert.deepEqual(visited.from("https://b.test"), [], "B's own pages did not start the visit");
  visited.observe("T", committed("https://c.test/"), "https://b.test/b/");
  assert.deepEqual(visited.from("https://c.test"), ["https://c.test/"], "a navigation no page started counts on its own origin");
  visited.observe("T", { method: "Page.navigatedWithinDocument", params: { frameId: "T", url: "https://c.test/#x" } });
  visited.observe("T", requested("https://d.test/", "newTab"), "https://c.test/");
  assert.deepEqual(visited.from("https://c.test"), ["https://c.test/", "https://c.test/#x", "https://d.test/"]);
  visited.observe("T", { method: "Page.navigatedWithinDocument", params: { frameId: "child", url: "https://c.test/#y" } });
  visited.observe("T", { method: "Page.frameNavigated", params: { frame: { id: "child", parentId: "T", url: "https://e.test/" } } });
  assert.deepEqual(visited.from("https://e.test"), [], "child frames do not count");
  visited.observe("T", committed("https://c.test/"), "https://c.test/");
  assert.equal(seen.length, 5, "only a new visit is announced");
});

test("MP-08/MP-10 protocol 593: the visited set keeps the most recent visits", () => {
  const visited = new VisitedLinks(2);
  for (const url of ["https://a.test/1", "https://a.test/2", "https://a.test/1", "https://a.test/3"]) visited.record("https://a.test", url);
  assert.deepEqual(visited.from("https://a.test"), ["https://a.test/1", "https://a.test/3"]);
  visited.record("null", "https://a.test/4"); visited.record("https://a.test", "data:text/html,x");
  assert.deepEqual(visited.from("https://a.test"), ["https://a.test/1", "https://a.test/3"], "opaque origins and non-web URLs are not kept");
});

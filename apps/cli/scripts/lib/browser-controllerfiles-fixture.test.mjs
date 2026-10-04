// MP-08/MP-10/MP-11: fixture proofs do not close provider or client acceptance.
import assert from "node:assert/strict";
import test from "node:test";
import { controllerfilesSamples, assertControllerfilesReceipt, startControllerfilesFixture } from "./browser-controllerfiles-fixture.mjs";

test("hidden chooser fixture receives exact UTF-8 and binary files with strict provenance", async t => {
  const fixture = await startControllerfilesFixture();
  t.after(() => fixture.close());
  const html = await fetch(fixture.origin).then(response => response.text());
  assert.match(html, /type="file"[^>]*hidden/);
  assert.match(html, /Choose documents/);
  assert.match(html, /input.click\(\)/);
  const expected = controllerfilesSamples();
  const form = new FormData();
  for (const file of expected) form.append("attachment", new Blob([file.bytes], { type: file.type }), file.name);
  const response = await fetch(`${fixture.origin}/upload`, { method: "POST", body: form });
  assert.equal(response.status, 200);
  const { attachments } = await response.json();
  assertControllerfilesReceipt(attachments, expected);
  assert.deepEqual(fixture.receipts, [attachments]);
  for (const changes of [{ name: "wrong.txt" }, { contentType: "application/pdf" }, { sizeBytes: 1 }, { sha256: "0".repeat(64) }]) {
    assert.throws(() => assertControllerfilesReceipt([{ ...attachments[0], ...changes }, ...attachments.slice(1)], expected), /provenance/);
  }
  assert.throws(() => assertControllerfilesReceipt(attachments.slice(1), expected), /count/);
});

test("malformed, missing and over-count upload requests never reuse partial receipt state", async t => {
  const fixture = await startControllerfilesFixture();
  t.after(() => fixture.close());
  for (const body of [new FormData(), "invalid", (() => {
    const form = new FormData();
    for (let i = 0; i < 21; i++) form.append("attachment", new Blob(["fixture"]), `${i}.txt`);
    return form;
  })()]) {
    const response = await fetch(`${fixture.origin}/upload`, { method: "POST", body });
    assert.equal(response.status, 400);
    assert.deepEqual(fixture.receipts, []);
  }
  const malformed = await fetch(`${fixture.origin}/upload`, { method: "POST",
    headers: { "content-type": "multipart/form-data; boundary=missing" }, body: "not a multipart request" });
  assert.equal(malformed.status, 400);
  assert.deepEqual(fixture.receipts, []);
  const oversized = await fetch(`${fixture.origin}/upload`, { method: "POST", body: "x".repeat(1024 * 1024 + 1) });
  assert.equal(oversized.status, 413);
  assert.deepEqual(fixture.receipts, []);
});

test("downloads have exact names, bytes and hashes and unknown paths stay missing", async t => {
  const fixture = await startControllerfilesFixture();
  t.after(() => fixture.close());
  for (const file of controllerfilesSamples()) {
    const response = await fetch(`${fixture.origin}/downloads/${file.name}`);
    assert.equal(response.status, 200);
    assert.equal(response.headers.get("content-disposition"), `attachment; filename="${file.name}"`);
    assert.equal(response.headers.get("content-type"), file.type);
    assert.deepEqual(Buffer.from(await response.arrayBuffer()), file.bytes);
  }
  assert.equal((await fetch(`${fixture.origin}/downloads/missing.txt`)).status, 404);
  assert.equal((await fetch(`${fixture.origin}/downloads/%2e%2e%2fprivate`)).status, 404);
});

test("network fixture records real cookie/auth presence and returns only bounded public receipts", async t => {
  const fixture = await startControllerfilesFixture();
  t.after(() => fixture.close());
  const response = await fetch(`${fixture.origin}/network-proof?private=synthetic`, {
    headers: { cookie: "fixture_private=synthetic", authorization: "Bearer synthetic" },
  });
  assert.equal(response.status, 200);
  assert.deepEqual(fixture.network, [{ method: "GET", path: "/network-proof", cookiePresent: true, authPresent: true }]);
  assert.deepEqual(await response.json(), { observed: true });
});

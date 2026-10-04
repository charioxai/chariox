import assert from "node:assert/strict";
import { mkdtemp, mkdir, readFile, rm, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { appPlaceholder, appRestoreOrigin, blockAppRestores, placeholderOrigin, prepareAppRestores } from "./browser-app-restore.mjs";

const int = value => { const b = Buffer.alloc(4); b.writeInt32LE(value); return b; };
function str(value) { const b = Buffer.from(value); return Buffer.concat([int(b.length), b, Buffer.alloc((4 - b.length % 4) % 4)]); }
function navigation(url) {
  const payload = Buffer.concat([int(42), int(3), str(url), str(""), str("encoded old origin PageState"), int(0), str(url)]);
  const header = Buffer.alloc(3); header.writeUInt16LE(payload.length + 5); header[2] = 6;
  return Buffer.concat([header, int(payload.length), payload]);
}
const header = Buffer.concat([Buffer.from("SNSS"), int(3)]);

for (const origin of ["https://app.todo-1.invalid", "https://todo-1.app.chariox.internal"]) {
  test(`saved ${origin} navigations become scriptless placeholders before restore`, () => {
    const other = navigation("https://example.test/path");
    const input = Buffer.concat([header, other, navigation(origin + "/draft?x=1")]);
    const safe = blockAppRestores(input);
    assert.deepEqual(safe.subarray(0, header.length + other.length), Buffer.concat([header, other]));
    assert.ok(safe.includes(Buffer.from(appPlaceholder(origin))));
    assert.equal(safe.includes(Buffer.from("encoded old origin PageState", "utf8"), header.length + other.length), false);
    assert.equal(safe.includes(Buffer.from(origin)), false);
    assert.deepEqual(blockAppRestores(safe), safe);
    assert.equal(placeholderOrigin(appPlaceholder(origin)), origin);
    assert.equal(safe.readUInt16LE(header.length + other.length), safe.length - header.length - other.length - 2);
  });
}

test("only exact App origins are placeholdered; a placeholder conveys no installation authority", () => {
  for (const url of ["https://example.invalid", "https://app.todo.invalid.evil.test", "http://app.todo.invalid", "https://app.todo.invalid:4433", "https://user@app.todo.invalid", "https://other.todo.app.chariox.internal"]) assert.equal(appRestoreOrigin(url), null);
  assert.equal(placeholderOrigin("data:text/html,reconnecting#chariox-app=https://app.todo.invalid"), null);
  assert.throws(() => appPlaceholder("https://evil.test"));
});

test("unsupported sessions fail closed without partial profile writes", async () => {
  const profile = await mkdtemp(path.join(os.tmpdir(), "chariox-app-restore-"));
  try {
    const directory = path.join(profile, "Default", "Sessions"); await mkdir(directory, {recursive:true});
    const file = path.join(directory, "Session_1");
    const bytes = Buffer.concat([header, navigation("https://app.todo.invalid/")]);
    await writeFile(file, bytes); await writeFile(path.join(directory, "Session_2"), Buffer.from("SNSSbad"));
    await assert.rejects(prepareAppRestores(profile), /Unsupported/);
    assert.deepEqual(await readFile(file), bytes);
    await rm(path.join(directory, "Session_2"));
    await prepareAppRestores(profile);
    assert.deepEqual(await readFile(file), blockAppRestores(bytes));
    assert.deepEqual(blockAppRestores(bytes.subarray(0, bytes.length - 1)), header);
    assert.throws(() => blockAppRestores(Buffer.concat([Buffer.from("SNSS"), int(4)])), /Unsupported/);
  } finally { await rm(profile, {recursive:true, force:true}); }
});

test("a crash-truncated tail preserves and protects preceding complete App records", () => {
  const complete = Buffer.concat([header, navigation("https://app.todo.invalid/")]);
  for (const tail of [Buffer.from([20]), navigation("https://example.test").subarray(0, 12)]) {
    assert.deepEqual(blockAppRestores(Buffer.concat([complete, tail])), blockAppRestores(complete));
  }
});

test("the owned offline launcher restores after a crash without changing other preferences", async () => {
  const profile = await mkdtemp(path.join(os.tmpdir(), "chariox-crash-restore-"));
  try {
    const directory = path.join(profile, "Default");
    await mkdir(path.join(directory, "Sessions"), { recursive: true });
    await writeFile(path.join(directory, "Sessions", "Session_1"), Buffer.concat([header, navigation("https://example.test/")]));
    const preferences = { profile: { exit_type: "Crashed", exited_cleanly: false, other: 42 }, session: { restore_on_startup: 5 }, other: { keep: true } };
    const file = path.join(directory, "Preferences");
    await writeFile(file, JSON.stringify(preferences));
    const { prepareChromiumLaunch } = await import("./browser-app-restore.mjs");
    assert.equal(await prepareChromiumLaunch(profile), "restore");
    preferences.profile.exit_type = "Normal";
    preferences.profile.exited_cleanly = true;
    assert.deepEqual(JSON.parse(await readFile(file, "utf8")), preferences);
  } finally { await rm(profile, { recursive: true, force: true }); }
});

test("unsupported sessions stay fenced on a crashed profile", async () => {
  const profile = await mkdtemp(path.join(os.tmpdir(), "chariox-crash-fenced-"));
  try {
    const directory = path.join(profile, "Default");
    await mkdir(path.join(directory, "Sessions"), { recursive: true });
    await writeFile(path.join(directory, "Sessions", "Session_1"), Buffer.from("unknown session"));
    const file = path.join(directory, "Preferences");
    const bytes = JSON.stringify({ profile: { exit_type: "Crashed", exited_cleanly: false } });
    await writeFile(file, bytes);
    const { prepareChromiumLaunch } = await import("./browser-app-restore.mjs");
    assert.equal(await prepareChromiumLaunch(profile), "fresh");
    assert.equal(await readFile(file, "utf8"), bytes);
  } finally { await rm(profile, { recursive: true, force: true }); }
});

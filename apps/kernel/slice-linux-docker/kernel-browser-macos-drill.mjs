// MD-2/MD-4: coordinator replay on macOS. No daemon socket or live profile is used.
// Usage: node <script> /absolute/chariox-kernel-tests /absolute/external/evidence
import assert from "node:assert/strict";
import { spawn, execFileSync } from "node:child_process";
import { access, copyFile, mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { createHash } from "node:crypto";
import os from "node:os";
import path from "node:path";
import { inflateSync } from "node:zlib";
async function executable(environment) {
  const names = ["Chromium.app/Contents/MacOS/Chromium", "Google Chrome.app/Contents/MacOS/Google Chrome", "Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing"];
  const candidates = environment.CHARIOX_KERNEL_BROWSER_EXECUTABLE ? [environment.CHARIOX_KERNEL_BROWSER_EXECUTABLE]
    : ["/Applications", path.join(os.homedir(), "Applications")].flatMap(root => names.map(name => path.join(root, name)));
  for (const candidate of candidates) {
    assert(path.isAbsolute(candidate), "MD-4: Chrome executable must be absolute");
    try { await access(candidate, 1); return candidate; } catch {}
  }
  throw new Error("MD-4: install native Chromium/Chrome or select a pinned executable");
}
function signalOwnedProcess(pid, signal) {
  assert(Number.isSafeInteger(pid) && pid > 1, "MD-4: refusing unsafe process ID");
  process.kill(pid, signal);
}

assert.equal(process.platform, "darwin", "MD-4: native Mac replay only");
const [binary, output] = process.argv.slice(2);
assert(binary && path.isAbsolute(binary) && output && path.isAbsolute(output), "MD-4: absolute test binary and external evidence directory required");
await access(binary, 1);
const chrome = await executable(process.env);
const parent = path.join(os.homedir(), ".chariox/dev/kbrowser-macos");
await mkdir(parent, { recursive: true, mode: 0o700 });
const root = await mkdtemp(path.join(parent, "drill-"));
// Unix socket paths must be short. This directory is disposable and lane-owned.
const temporary = await mkdtemp("/tmp/md4-");
await mkdir(output, { recursive: true, mode: 0o700 });
await mkdir(path.join(root, "home/chariox"), { recursive: true, mode: 0o700 });
const hash = file => readFile(file).then(bytes => createHash("sha256").update(bytes).digest("hex"));
const receipt = { item: "MD-2/MD-4/MD-5", binary_sha256: await hash(binary), started: new Date().toISOString(), result: "RED", samples: [] };
// Track only this replay's descendants and exact private paths. Keep process
// start time as well as PID so cleanup cannot target a reused PID.
let child;
const owned = new Map();
const ownedProcesses = () => {
  const rows = execFileSync("ps", ["-axo", "pid=,ppid=,lstart=,command="], { encoding: "utf8" }).split("\n").flatMap(line => {
    const match = line.match(/^\s*(\d+)\s+(\d+)\s+(\S+\s+\S+\s+\d+\s+\S+\s+\d+)\s+(.*)$/);
    return match ? [{ pid: +match[1], parent: +match[2], identity: `${match[3]} ${match[4]}`, command: match[4] }] : [];
  });
  const selected = new Set(rows.filter(row => row.pid > 1 && row.pid !== process.pid &&
    ((row.pid === child?.pid && child.exitCode === null && child.signalCode === null) || owned.get(row.pid) === row.identity || row.command.includes(root) || row.command.includes(temporary))).map(row => row.pid));
  let previous;
  do {
    previous = selected.size;
    for (const row of rows) if (row.pid > 1 && selected.has(row.parent)) selected.add(row.pid);
  } while (selected.size !== previous);
  const current = rows.filter(row => selected.has(row.pid));
  for (const row of current) owned.set(row.pid, row.identity);
  return current;
};
const sample = () => {
  const read = (command, args) => { try { return execFileSync(command, args, { encoding: "utf8", timeout: 3000 }); } catch { return "unavailable"; } };
  receipt.samples.push({ at: new Date().toISOString(), disk: read("df", ["-k", "/"]), memory: read("vm_stat", []), pressure: read("memory_pressure", ["-Q"]), swap: read("sysctl", ["vm.swapusage"]) });
  ownedProcesses();
};
try {
  sample();
  const nodeDirectory = path.dirname(process.execPath);
  child = spawn(binary, ["--ignored", "--exact", "runtime::router::tests::kernel_browser::kernel_browser_linux_integration_drill"], {
    env: { PATH: `${nodeDirectory}:/usr/bin:/bin:/usr/sbin:/sbin`, HOME: path.join(root, "home"), TMPDIR: temporary,
      CHARIOX_HOME: path.join(root, "home/chariox"), CHARIOX_MD4_DRILL_ROOT: root,
      CHARIOX_KERNEL_BROWSER_EXECUTABLE: chrome, CHARIOX_BROWSER_CONTROLLER_NODE: process.execPath,
      CHARIOX_MD4_PHASE: "", RUST_BACKTRACE: "0" }, stdio: "inherit",
  });
  const sampling = setInterval(sample, 5000);
  const timeout = setTimeout(() => {
    if (Number.isSafeInteger(child?.pid) && child.pid > 1 && child.exitCode === null && child.signalCode === null) child.kill("SIGTERM");
  }, 180_000);
  const status = await new Promise((resolve, reject) => { child.once("error", reject); child.once("exit", (code, signal) => resolve({ code, signal })); }).finally(() => { clearInterval(sampling); clearTimeout(timeout); });
  receipt.exit = status;
  assert.equal(status.code, 0, "MD-4: Mac kernel replay failed");
  await copyFile(path.join(root, "screenshot.png"), path.join(output, "screenshot.png"));
  await copyFile(path.join(root, "MD5-masked.png"), path.join(output, "MD5-masked.png"));
  // Protected frames use the bounded RGBA/filter-zero encoder. Inspect pixels
  // independently of the kernel test's success receipt; never print image bytes.
  const png = await readFile(path.join(output, "MD5-masked.png")), parts = [];
  assert.equal(png.readUInt32BE(16), 1280); assert.equal(png.readUInt32BE(20), 800);
  assert.equal(png[25], 6);
  for (let offset = 8; offset < png.length;) {
    const length = png.readUInt32BE(offset);
    if (png.toString("ascii", offset + 4, offset + 8) === "IDAT") parts.push(png.subarray(offset + 8, offset + 8 + length));
    offset += length + 12;
  }
  const pixels = inflateSync(Buffer.concat(parts), { maxOutputLength: 800 * (1280 * 4 + 1) });
  const black = (x, y) => pixels.subarray(y * (1280 * 4 + 1) + 1 + x * 4, y * (1280 * 4 + 1) + 1 + x * 4 + 4).equals(Buffer.from([0, 0, 0, 255]));
  receipt.pixel_checks = { password_masked: black(80, 200), echo_masked: black(20, 108), retained_background: !black(600, 500) };
  assert(receipt.pixel_checks.password_masked && receipt.pixel_checks.echo_masked, "MD-5: fixture field and plaintext echo pixels must be opaque");
  receipt.assertions = await readFile(path.join(root, "MD4-PASS.txt"), "utf8");
  receipt.result = "PASS";
} finally {
  // Also settle this replay's processes if its timeout interrupted normal cleanup.
  for (const signal of ["SIGTERM", "SIGKILL"]) {
    const remaining = ownedProcesses();
    for (const row of remaining) { try { signalOwnedProcess(row.pid, signal); } catch (error) { if (error.code !== "ESRCH") throw error; } }
    if (remaining.length) await new Promise(resolve => setTimeout(resolve, signal === "SIGTERM" ? 3000 : 1000));
  }
  const remaining = ownedProcesses();
  receipt.cleanup = remaining.length === 0 ? "PASS" : "RED: owned processes remain; state retained";
  if (remaining.length === 0) { await rm(root, { recursive: true, force: true }); await rm(temporary, { recursive: true, force: true }); }
  else receipt.result = "RED";
  sample();
  receipt.finished = new Date().toISOString();
  await writeFile(path.join(output, "receipt.json"), JSON.stringify(receipt, null, 2));
  assert.equal(receipt.cleanup, "PASS", "MD-4: owned process cleanup failed");
}

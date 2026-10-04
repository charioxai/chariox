// MD-2/MD-4: coordinator replay on macOS. No daemon socket or live profile is used.
// Usage: node <script> /absolute/chariox-kernel-tests /absolute/external/evidence
import assert from "node:assert/strict";
import { spawn, execFileSync } from "node:child_process";
import { access, copyFile, mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { createHash } from "node:crypto";
import os from "node:os";
import path from "node:path";
import { executable } from "./docker/kernel-browser-process.mjs";

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
const receipt = { item: "MD-2/MD-4", binary_sha256: await hash(binary), started: new Date().toISOString(), result: "RED", samples: [] };
const sample = () => {
  const read = (command, args) => { try { return execFileSync(command, args, { encoding: "utf8", timeout: 3000 }); } catch { return "unavailable"; } };
  receipt.samples.push({ at: new Date().toISOString(), disk: read("df", ["-k", "/"]), memory: read("vm_stat", []), pressure: read("memory_pressure", ["-Q"]), swap: read("sysctl", ["vm.swapusage"]) });
};
let child;
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
  const timeout = setTimeout(() => child.kill("SIGTERM"), 180_000);
  const status = await new Promise((resolve, reject) => { child.once("error", reject); child.once("exit", (code, signal) => resolve({ code, signal })); }).finally(() => { clearInterval(sampling); clearTimeout(timeout); });
  receipt.exit = status;
  assert.equal(status.code, 0, "MD-4: Mac kernel replay failed");
  await copyFile(path.join(root, "screenshot.png"), path.join(output, "screenshot.png"));
  receipt.assertions = await readFile(path.join(root, "MD4-PASS.txt"), "utf8");
  receipt.result = "PASS";
} finally {
  // Exact owned paths only. The test settles its controller/Chrome on failure.
  // Refuse removal if any process still advertises this disposable profile root.
  const processes = execFileSync("ps", ["-axo", "command="], { encoding: "utf8" });
  const remaining = processes.split("\n").filter(line => line.includes(root) || line.includes(temporary));
  receipt.cleanup = remaining.length === 0 ? "PASS" : "RED: owned processes remain; state retained";
  if (remaining.length === 0) { await rm(root, { recursive: true, force: true }); await rm(temporary, { recursive: true, force: true }); }
  else receipt.result = "RED";
  sample();
  receipt.finished = new Date().toISOString();
  await writeFile(path.join(output, "receipt.json"), JSON.stringify(receipt, null, 2));
  assert.equal(receipt.cleanup, "PASS", "MD-4: owned process cleanup failed");
}

import { constants } from "node:fs";
import { lstat, mkdir, open, realpath, utimes } from "node:fs/promises";
import { createHash, randomUUID } from "node:crypto";
import { execFile } from "node:child_process";
import { fileURLToPath } from "node:url";
import { tmpdir } from "node:os";
import path from "node:path";
import { assertNotCancelled } from "./browser-controller-actions.mjs";

const MAX_BYTES = 1024 * 1024 * 1024;
const MAX_FILES = 128;
const COPY_CHUNK_BYTES = 64 * 1024;
export function uploadCopyTimeoutMs(bytes) {
  // Slow VPS storage receives a byte-scaled allowance, still capped and
  // subordinate to the caller's cancellation/deadline.
  return Math.min(120_000, 5_000 + Math.ceil(bytes * 1000 / (4 * 1024 * 1024)));
}

function denied(message) {
  return Object.assign(new Error(message), { code: "browser_upload_staging_unavailable" });
}

async function resolveOpenedFile(source) {
  if (process.platform !== "linux") throw denied("secure browser upload staging requires Linux descriptor resolution");
  return realpath(`/proc/self/fd/${source.fd}`);
}

// Chromium's File objects can outlive the controller and CDP connection. Once
// dispatch starts, this reservation is deliberately durable, including on an
// ambiguous command failure. No transport event proves those files disposable.
export class BrowserUploadStaging {
  constructor({ root, maximumBytes = MAX_BYTES, maximumFiles = MAX_FILES, now = () => performance.now(), descriptorPath = resolveOpenedFile } = {}) {
    this.root = root;
    this.maximumBytes = Math.min(maximumBytes, MAX_BYTES);
    this.maximumFiles = Math.min(maximumFiles, MAX_FILES);
    this.now = now;
    this.descriptorPath = descriptorPath;
    if (!Number.isSafeInteger(this.maximumBytes) || this.maximumBytes < 1
      || !Number.isSafeInteger(this.maximumFiles) || this.maximumFiles < 1) throw denied("invalid upload staging limits");
  }

  async directory() {
    const root = this.root ?? path.join(await realpath(tmpdir()), `chariox-browser-uploads-${process.getuid()}`);
    await mkdir(root, { mode: 0o700 }).catch(error => { if (error.code !== "EEXIST") throw error; });
    const status = await lstat(root);
    if (!status.isDirectory() || status.isSymbolicLink() || status.uid !== process.getuid()
      || (status.mode & 0o777) !== 0o700 || await realpath(root) !== root) {
      throw denied("upload staging root must be a canonical private user-owned directory");
    }
    return root;
  }

  async transaction(request) {
    const root = await this.directory();
    await new Promise((resolve, reject) => {
      const child = execFile("python3", [fileURLToPath(new URL("./browser-upload-store.py", import.meta.url)), root],
        { timeout: 2000, killSignal: "SIGKILL", maxBuffer: 8192 }, (error, _output, stderr) => {
          if (!error) resolve();
          else reject(denied(stderr.includes("quota is full") ? "upload staging quota is full; verified browser retirement is required"
            : "upload staging transaction unavailable; retained data requires reconciliation"));
        });
      child.stdin.on("error", () => {});
      child.stdin.end(JSON.stringify(request));
    });
    return root;
  }

  async prepare({ files, metadata, browserIdentity, signal, connection }) {
    if (!Array.isArray(files) || files.length < 1 || files.length > 20
      || !Array.isArray(metadata) || metadata.length !== files.length
      || metadata.some(item => !item || !Number.isSafeInteger(item.size) || item.size < 0)
      || metadata.reduce((sum, item) => sum + item.size, 0) > 512 * 1024 * 1024) {
      throw denied("upload staging requires a bounded approved file set");
    }
    if (typeof browserIdentity !== "string" || !/^wss?:\/\/[^\s]+\/devtools\/browser\/[A-Za-z0-9-]+$/.test(browserIdentity)) {
      throw denied("upload staging requires an observed physical browser identity");
    }
    const bytes = metadata.reduce((sum, item) => sum + item.size, 0);
    const deadline = this.now() + uploadCopyTimeoutMs(bytes);
    const check = () => {
      assertNotCancelled(signal);
      if (this.now() >= deadline) throw denied("upload staging exceeded its bounded copy deadline");
    };
    check();
    const id = randomUUID();
    const browser = createHash("sha256").update(browserIdentity).digest("hex");
    const processes = await connection?.send("SystemInfo.getProcessInfo").catch(() => undefined);
    const browserPid = processes?.processInfo?.find(item => item.type === "browser")?.id;
    const root = await this.transaction({ action: "reserve", entry: { id, browser, bytes, count: files.length },
      maximumBytes: this.maximumBytes, maximumFiles: this.maximumFiles,
      browserPid, browserPort: Number(new URL(browserIdentity).port) });
    const leaseRoot = path.join(root, id);
    let exposed = false;
    const discard = async () => {
      if (exposed) return;
      await this.transaction({ action: "discard", id });
    };
    try {
      await mkdir(leaseRoot, { mode: 0o700 });
      const staged = [];
      for (let index = 0; index < files.length; index += 1) {
        check();
        const source = await open(files[index], constants.O_RDONLY | constants.O_NOFOLLOW | constants.O_NONBLOCK);
        try {
          // O_NOFOLLOW covers the leaf; reject changed ancestor resolution too.
          // Linux procfs binds this check to the opened descriptor, not a second
          // lookup through attacker-changeable parent directories.
          const openedPath = await this.descriptorPath(source, files[index]);
          if (openedPath !== files[index]) throw denied("upload source path changed after authorization");
          const before = await source.stat({ bigint: true });
          const approved = metadata[index];
          if (!before.isFile() || before.dev !== approved.dev || before.ino !== approved.ino
            || Number(before.size) !== approved.size || before.mtimeNs !== approved.mtimeNs
            || before.ctimeNs !== approved.ctimeNs) throw denied("upload source changed after authorization");
          const slot = path.join(leaseRoot, String(index));
          await mkdir(slot, { mode: 0o700 });
          const destination = path.join(slot, path.basename(files[index]));
          const output = await open(destination, "wx", 0o400);
          try {
            const buffer = Buffer.alloc(COPY_CHUNK_BYTES);
            let copied = 0;
            while (copied < approved.size) {
              check();
              const { bytesRead } = await source.read(buffer, 0, Math.min(buffer.length, approved.size - copied), copied);
              if (bytesRead === 0) throw denied("upload source changed while staging");
              let written = 0;
              while (written < bytesRead) {
                check();
                const result = await output.write(buffer, written, bytesRead - written, copied + written);
                if (!result.bytesWritten) throw denied("upload staging write made no progress");
                written += result.bytesWritten;
              }
              copied += bytesRead;
            }
            const after = await source.stat({ bigint: true });
            if (["dev", "ino", "size", "mtimeNs", "ctimeNs"].some(key => before[key] !== after[key])) {
              throw denied("upload source changed while staging");
            }
            check();
            await output.sync();
          } finally { await output.close(); }
          const lastModified = new Date(Number(approved.mtimeNs / 1000000n));
          await utimes(destination, lastModified, lastModified);
          staged.push(destination);
        } finally { await source.close(); }
      }
      check();
      return { files: staged, markExposed: async () => {
        await this.transaction({ action: "expose", id });
        exposed = true;
      }, discard };
    } catch (error) {
      await discard();
      if (error.code === "browser_action_cancelled" || error.code === "browser_upload_staging_unavailable") throw error;
      throw denied("upload staging failed before files were exposed");
    }
  }
}

const defaultStaging = new BrowserUploadStaging();
export const stageBrowserUploadFiles = options => defaultStaging.prepare(options);

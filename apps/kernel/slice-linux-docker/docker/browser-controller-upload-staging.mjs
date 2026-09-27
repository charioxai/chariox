import { constants } from "node:fs";
import { lstat, mkdir, open, readdir, realpath, rename, rm, utimes } from "node:fs/promises";
import { createHash, randomUUID } from "node:crypto";
import { tmpdir } from "node:os";
import path from "node:path";
import { assertNotCancelled } from "./browser-controller-actions.mjs";

const MAX_BYTES = 1024 * 1024 * 1024;
const MAX_FILES = 128;
const MAX_LEASES = 128;
const COPY_CHUNK_BYTES = 64 * 1024;
const COPY_TIMEOUT_MS = 5_000;

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

  async transaction(operation) {
    const root = await this.directory();
    const lock = path.join(root, "lock");
    try { await mkdir(lock, { mode: 0o700 }); }
    catch { throw denied("upload staging is busy or requires cleanup after an interrupted operation"); }
    try {
      const ledgerPath = path.join(root, "ledger.json");
      let ledger;
      try {
        const handle = await open(ledgerPath, constants.O_RDONLY | constants.O_NOFOLLOW | constants.O_NONBLOCK);
        try {
          const status = await handle.stat();
          if (!status.isFile() || status.uid !== process.getuid() || (status.mode & 0o777) !== 0o600
            || status.size > 65536) throw denied("invalid upload staging ledger");
          ledger = JSON.parse(await handle.readFile("utf8"));
        } finally { await handle.close(); }
      } catch (error) {
        if (error.code !== "ENOENT") throw denied("upload staging ledger requires reconciliation");
        if ((await readdir(root)).some(name => name !== "lock")) throw denied("upload staging ledger is missing with retained files");
        ledger = [];
      }
      if (!Array.isArray(ledger) || ledger.length > MAX_LEASES || ledger.some(entry =>
        !entry || !/^[a-f0-9-]{36}$/.test(entry.id) || !/^[a-f0-9]{64}$/.test(entry.browser)
        || !Number.isSafeInteger(entry.bytes) || entry.bytes < 0 || entry.bytes > MAX_BYTES
        || !Number.isSafeInteger(entry.count) || entry.count < 1 || entry.count > 20)) {
        throw denied("upload staging ledger requires reconciliation");
      }
      const result = await operation(root, ledger);
      const replacement = path.join(root, `ledger-${randomUUID()}.tmp`);
      try {
        const handle = await open(replacement, "wx", 0o600);
        try { await handle.writeFile(JSON.stringify(ledger)); await handle.sync(); }
        finally { await handle.close(); }
        await rename(replacement, ledgerPath);
        const directory = await open(root, constants.O_RDONLY);
        try { await directory.sync(); } finally { await directory.close(); }
      } finally { await rm(replacement, { force: true }); }
      return result;
    } finally { await rm(lock, { recursive: true }); }
  }

  async prepare({ files, metadata, browserIdentity, signal }) {
    if (!Array.isArray(files) || files.length < 1 || files.length > 20
      || !Array.isArray(metadata) || metadata.length !== files.length
      || metadata.some(item => !item || !Number.isSafeInteger(item.size) || item.size < 0)
      || metadata.reduce((sum, item) => sum + item.size, 0) > 512 * 1024 * 1024) {
      throw denied("upload staging requires a bounded approved file set");
    }
    if (typeof browserIdentity !== "string" || !/^wss?:\/\/[^\s]+\/devtools\/browser\/[A-Za-z0-9-]+$/.test(browserIdentity)) {
      throw denied("upload staging requires an observed physical browser identity");
    }
    const deadline = this.now() + COPY_TIMEOUT_MS;
    const check = () => {
      assertNotCancelled(signal);
      if (this.now() >= deadline) throw denied("upload staging exceeded its bounded copy deadline");
    };
    check();
    const bytes = metadata.reduce((sum, item) => sum + item.size, 0);
    const id = randomUUID();
    const browser = createHash("sha256").update(browserIdentity).digest("hex");
    const root = await this.transaction((root, ledger) => {
      if (ledger.length >= MAX_LEASES || ledger.reduce((sum, item) => sum + item.bytes, bytes) > this.maximumBytes
        || ledger.reduce((sum, item) => sum + item.count, files.length) > this.maximumFiles) {
        throw denied("upload staging quota is full; retained browser File objects require verified browser retirement before cleanup");
      }
      ledger.push({ id, browser, bytes, count: files.length });
      return root;
    });
    const leaseRoot = path.join(root, id);
    let exposed = false;
    const discard = async () => {
      if (exposed) return;
      await this.transaction(async (_root, ledger) => {
        await rm(leaseRoot, { recursive: true, force: true });
        const index = ledger.findIndex(entry => entry.id === id);
        if (index !== -1) ledger.splice(index, 1);
      });
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
          if (!before.isFile() || Number(before.dev) !== approved.dev || Number(before.ino) !== approved.ino
            || Number(before.size) !== approved.size || Number(before.mtimeNs / 1000000n) !== Math.trunc(approved.mtimeMs)
            || Number(before.ctimeNs / 1000000n) !== Math.trunc(approved.ctimeMs)) throw denied("upload source changed after authorization");
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
          await utimes(destination, new Date(approved.mtimeMs), new Date(approved.mtimeMs));
          staged.push(destination);
        } finally { await source.close(); }
      }
      check();
      return { files: staged, markExposed() { exposed = true; }, discard };
    } catch (error) {
      await discard();
      if (error.code === "browser_action_cancelled" || error.code === "browser_upload_staging_unavailable") throw error;
      throw denied("upload staging failed before files were exposed");
    }
  }
}

const defaultStaging = new BrowserUploadStaging();
export const stageBrowserUploadFiles = options => defaultStaging.prepare(options);

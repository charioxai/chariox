// Chromium restores session URLs before CDP is available. App documents need
// Fetch interception, so put only their saved navigations behind a scriptless
// placeholder before launching Chromium. Never edit a running profile.
import { mkdir, readdir, readFile, rename, writeFile } from "node:fs/promises";
import path from "node:path";
import { pathToFileURL } from "node:url";

const PAGE = Buffer.from('<!doctype html><meta charset="utf-8"><meta http-equiv="Content-Security-Policy" content="default-src \'none\'"><title>App reconnecting</title><p>Reconnecting App…</p>').toString("base64");
const PREFIX = `data:text/html;base64,${PAGE}#chariox-app=`;
class RestoreFormatError extends Error {}

export function appRestoreOrigin(url) {
  try {
    const parsed = new URL(url);
    return parsed.protocol === "https:" && !parsed.port && !parsed.username && !parsed.password
      && (/^app\.[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?\.invalid$/.test(parsed.hostname)
        || /^[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?\.app\.chariox\.internal$/.test(parsed.hostname))
      ? parsed.origin : null;
  } catch { return null; }
}

export function placeholderOrigin(url) {
  if (typeof url !== "string" || !url.startsWith(PREFIX)) return null;
  try { return appRestoreOrigin(decodeURIComponent(url.slice(PREFIX.length))); } catch { return null; }
}

export function appPlaceholder(origin) {
  if (appRestoreOrigin(origin) !== origin) throw new Error("Invalid App restore origin");
  return PREFIX + encodeURIComponent(origin);
}

function int(value) {
  const bytes = Buffer.alloc(4);
  bytes.writeInt32LE(value);
  return bytes;
}
function string(bytes, length = bytes.length) {
  return Buffer.concat([int(length), bytes, Buffer.alloc((4 - bytes.length % 4) % 4)]);
}

// SNSS v1/v3: uint16 record size, uint8 command, then a base::Pickle.
// UpdateTabNavigation (6) starts with tab id, navigation index and URL.
// See Chromium components/sessions/core/{session_service_commands,
// base_session_service_commands,serialized_navigation_entry}.cc. For an App
// replace the entire navigation (including encoded PageState/original URL),
// preserving tab/window/index commands and every non-App record byte for byte.
export function blockAppRestores(bytes) {
  if (bytes.length < 8 || bytes.subarray(0, 4).toString() !== "SNSS"
    || ![1, 3].includes(bytes.readInt32LE(4))) throw new RestoreFormatError("Unsupported Chromium session format");
  const records = [bytes.subarray(0, 8)];
  for (let offset = 8; offset < bytes.length;) {
    // Like Chromium, retain complete records when a crash cut the final write.
    if (offset + 2 > bytes.length) break;
    const size = bytes.readUInt16LE(offset);
    const end = offset + 2 + size;
    if (end > bytes.length) break;
    if (size < 1) throw new RestoreFormatError("Invalid Chromium session record");
    let record = bytes.subarray(offset, end);
    if (record[2] === 6) {
      const pickle = record.subarray(3);
      if (pickle.length < 16 || pickle.readUInt32LE(0) !== pickle.length - 4) throw new RestoreFormatError("Invalid Chromium navigation pickle");
      const length = pickle.readInt32LE(12);
      if (length < 0 || 16 + length > pickle.length) throw new RestoreFormatError("Invalid Chromium navigation URL");
      const origin = appRestoreOrigin(pickle.subarray(16, 16 + length).toString());
      if (origin) {
        const title = Buffer.from("App reconnecting", "utf16le");
        const payload = Buffer.concat([pickle.subarray(4, 12),
          string(Buffer.from(appPlaceholder(origin))), string(title, title.length / 2),
          string(Buffer.alloc(0)), int(0)]);
        const header = Buffer.alloc(3);
        header.writeUInt16LE(1 + 4 + payload.length);
        header[2] = 6;
        record = Buffer.concat([header, int(payload.length), payload]);
      }
    }
    records.push(record);
    offset = end;
  }
  return Buffer.concat(records);
}

async function sessionFiles(profile) {
  const directory = path.join(profile, "Default");
  let names = [];
  try { names = (await readdir(path.join(directory, "Sessions"))).filter(name => /^Session_\d+$/.test(name)).map(name => path.join("Sessions", name)); }
  catch (error) { if (error.code !== "ENOENT") throw error; }
  return [...names, "Last Session", "Current Session"].map(name => path.join(directory, name));
}

export async function prepareAppRestores(profile) {
  const changes = [];
  for (const file of await sessionFiles(profile)) {
    let bytes;
    try { bytes = await readFile(file); } catch (error) { if (error.code === "ENOENT") continue; throw error; }
    if (!bytes.length) continue;
    if (bytes.length > 64 * 1024 * 1024) throw new RestoreFormatError("Chromium session exceeds restore safety limit");
    const safe = blockAppRestores(bytes);
    if (!bytes.equals(safe)) changes.push([file, safe]);
  }
  // Validate all files first, then atomically replace each complete session.
  for (const [file, bytes] of changes) {
    const temporary = `${file}.chariox-restore-${process.pid}`;
    await writeFile(temporary, bytes, { mode: 0o600, flag: "wx" });
    await rename(temporary, file);
  }
}

export async function prepareChromiumLaunch(profile) {
  try {
    await prepareAppRestores(profile);
    return "restore";
  } catch (error) {
    if (!(error instanceof RestoreFormatError)) throw error;
    // Unknown/encrypted/corrupt sessions cannot be inspected for App origins.
    // Keep them for recovery outside Chromium's restore search, and launch a
    // fresh session. Merely omitting --restore-last-session is insufficient:
    // Chromium can also restore from its startup preferences.
    const backup = path.join(profile, "Default", `chariox-unrestorable-${Date.now()}-${process.pid}`);
    await mkdir(backup, { mode: 0o700 });
    for (const file of await sessionFiles(profile)) {
      try { await rename(file, path.join(backup, path.basename(file))); }
      catch (failure) { if (failure.code !== "ENOENT") throw failure; }
    }
    process.stderr.write(`[app-restore] ${error.message}; saved sessions retained in ${backup}; starting fresh\n`);
    return "fresh";
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  process.stdout.write(await prepareChromiumLaunch(process.argv[2]) + "\n");
}

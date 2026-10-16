#!/usr/bin/env node
// MP-08/MP-10/MP-11: export opaque home request spans; discard all other stderr.
import {open, mkdir, realpath} from 'node:fs/promises';
import path from 'node:path';
import {fileURLToPath} from 'node:url';

const PREFIX = 'MP-10-PACKET-TIMING ';
const STAGES = new Set(['kernel_receive', 'kernel_request_end']);
const MAX_LINE = 4096;
const MAX_OUTPUT = 64 * 1024 * 1024;

export function homeRequestTimingRow(line) {
  if (!line.startsWith(PREFIX)) return null;
  try {
    const value = JSON.parse(line.slice(PREFIX.length));
    if (!value || Object.keys(value).sort().join(',') !== 'at_ms,packet,stage'
      || !STAGES.has(value.stage)
      || !Number.isSafeInteger(value.packet) || value.packet < 0 || value.packet >= 2 ** 52
      || !Number.isFinite(value.at_ms) || value.at_ms <= 0) return null;
    return {stage: value.stage, packet: value.packet, at_ms: value.at_ms};
  } catch { return null; }
}

export function homeRequestTimingSummary(text) {
  // A live collector can be midway through its last write when the viewer
  // finishes. Inspect complete records only; never parse or copy the tail.
  const complete = text.slice(0, text.lastIndexOf('\n') + 1);
  const rows = complete.split('\n').filter(Boolean).map(line => homeRequestTimingRow(PREFIX + line));
  if (!rows.length || rows.some(row => !row)) throw new Error('MP-11: invalid home timing evidence');
  const starts = new Map(rows.filter(row => row.stage === 'kernel_receive').map(row => [row.packet, row.at_ms]));
  const ends = rows.filter(row => row.stage === 'kernel_request_end' && starts.has(row.packet));
  if (!ends.length || ends.some(row => row.at_ms < starts.get(row.packet)))
    throw new Error('MP-10: missing or invalid home receive/end span');
  return {rows: rows.length, completed: ends.length, partial_tail_ignored: complete.length !== text.length};
}

// A pipe can contain credentials in unrelated diagnostics. Never retain raw
// lines, prefixes, exception messages, commands, identities or authentication.
export async function captureHomeRequestTiming(input, directory, {maxBytes = MAX_OUTPUT} = {}) {
  if (!path.isAbsolute(directory) || !Number.isSafeInteger(maxBytes) || maxBytes < 0 || maxBytes > MAX_OUTPUT)
    throw new Error('MP-11: invalid home timing output options');
  const checkout = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../../..');
  const inCheckout = root => root === checkout || root.startsWith(checkout + path.sep);
  if (inCheckout(path.resolve(directory)))
    throw new Error('MP-11: home timing evidence must be outside the checkout');
  await mkdir(directory, {recursive: true, mode: 0o700});
  const root = await realpath(directory);
  if (inCheckout(root))
    throw new Error('MP-11: home timing evidence must be outside the checkout');
  const output = await open(path.join(root, 'home-request-timing.jsonl'), 'wx', 0o600);
  const stats = {receive: 0, end: 0, discarded: 0, oversized: 0, bytes: 0, capped: false};
  let pending = '', dropping = false;
  function accept(line, rows) {
    const row = homeRequestTimingRow(line);
    if (!row) {stats.discarded++; return;}
    const encoded = JSON.stringify(row) + '\n';
    const bytes = Buffer.byteLength(encoded);
    if (stats.bytes + bytes > maxBytes) {stats.capped = true; return;}
    stats.bytes += bytes;
    stats[row.stage === 'kernel_receive' ? 'receive' : 'end']++;
    rows.push(encoded);
  }
  try {
    for await (const chunk of input) {
      // The accepted format is ASCII. A UTF-8 split in unrelated stderr cannot
      // affect a valid timing mark, and never gets copied to the evidence.
      const text = chunk.toString();
      const rows = [];
      let start = 0;
      while (start < text.length) {
        const newline = text.indexOf('\n', start);
        const end = newline < 0 ? text.length : newline;
        if (!dropping && pending.length + end - start > MAX_LINE) {
          stats.oversized++; dropping = true; pending = '';
        }
        if (!dropping) pending += text.slice(start, end);
        if (newline < 0) break;
        if (!dropping) accept(pending, rows);
        pending = ''; dropping = false; start = newline + 1;
      }
      if (rows.length) await output.writeFile(rows.join(''));
    }
    if (pending && !dropping) {
      const rows = []; accept(pending, rows);
      if (rows.length) await output.writeFile(rows.join(''));
    }
  } finally { await output.close(); }
  return stats;
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    if (process.argv.length !== 3) throw new Error();
    const stats = await captureHomeRequestTiming(process.stdin, process.argv[2]);
    // Missing/capped spans are an instrumentation failure, never a product PASS.
    console.log(JSON.stringify({items: ['MP-08', 'MP-10', 'MP-11'], ...stats}));
    if (!stats.receive || !stats.end || stats.capped) process.exitCode = 1;
  } catch {
    console.error('MP-08/MP-10/MP-11: home timing capture failed; no raw diagnostics exported');
    process.exitCode = 1;
  }
}

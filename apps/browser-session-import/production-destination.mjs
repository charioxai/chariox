import {createHash,randomBytes} from 'node:crypto';
import {constants} from 'node:fs';
import {chmod,mkdir,open,unlink} from 'node:fs/promises';
import path from 'node:path';

import {applyControllerCookieImport} from './controller-cookie-import.mjs';
import {completeCookieImport} from './cookie-import-completion.mjs';
import {openCookieImportJournal} from './cookie-import-journal.mjs';

export async function applyProductionBrowserImport({controller,params,home = process.env.CHARIOX_HOME}) {
  const binding = validateBinding(params?.binding);
  const source = parsePayload(params?.payload_json);
  const scope = {
    sourceStoreId:text(params?.source_store_id),
    approvedDomains:textList(params?.domains,false),
    approvedPartitionSites:textList(params?.partition_sites,true),
  };
  if (typeof home !== 'string' || !path.isAbsolute(home) || !Number.isSafeInteger(params?.browser_generation)
      || params.browser_generation < 1 || typeof params?.overwrite !== 'boolean') fail();
  const journalDirectory = path.join(home,'browser-import',digest(binding.environment_id));
  let journal, key;
  try {
    await secureDirectory(journalDirectory);
    const completedPath = path.join(journalDirectory,'cookie-import.completed');
    await unlink(completedPath).catch(error => { if (error?.code !== 'ENOENT') throw error; });
    key = await journalKey(journalDirectory);
    journal = await openCookieImportJournal({directory:journalDirectory,key,binding:{
      userId:binding.user_id,roomId:binding.room_id,environmentId:binding.environment_id,
    }});
    const expected = {sourceStoreId:scope.sourceStoreId,approvedDomains:scope.approvedDomains,
      approvedPartitionSites:scope.approvedPartitionSites,overwrite:params.overwrite};
    const result = await applyControllerCookieImport({controller,
      browserGeneration:controller.browserGeneration,targetId:text(params.target_id),
      documentId:text(params.document_id),source,scope,overwrite:params.overwrite}, {
      authorize:current => controllerBindingMatches(current,expected,controller.browserGeneration),
      runExclusive:operation => operation(),journal,
      complete:async ({receipt}) => completeCookieImport({receipt,journal,
        recordOutcome:() => writeOutcome(completedPath,binding.request_id,receipt),
        clearPending:async () => {}}),
    });
    return {status:'applied',cookie_count:result.cookieCount};
  } catch (error) {
    if (error?.recoveryRequired === false) return {status:'rolled_back',cookie_count:0};
    fail();
  } finally {
    for (const cookie of source) if (cookie && typeof cookie === 'object') cookie.value = '';
    key?.fill(0);
    try { await journal?.close(); } catch { /* Retain the fixed import result. */ }
  }
}

function validateBinding(binding) {
  if (!binding || typeof binding !== 'object' || Array.isArray(binding)) fail();
  const result = {};
  for (const field of ['request_id','user_id','room_id','environment_id']) result[field] = text(binding[field]);
  if (!/^[a-fA-F0-9]{32}$/.test(result.request_id)) fail();
  return result;
}

function parsePayload(value) {
  if (typeof value !== 'string' || Buffer.byteLength(value) > 512 * 1024) fail();
  let source;
  try { source = JSON.parse(value); } catch { fail(); }
  if (!Array.isArray(source) || source.length > 512) fail();
  return source;
}

function controllerBindingMatches(current,expected,generation) {
  return current?.browserGeneration === generation && current?.sourceStoreId === expected.sourceStoreId
    && current?.overwrite === expected.overwrite
    && same(current?.approvedDomains,expected.approvedDomains)
    && same(current?.approvedPartitionSites,expected.approvedPartitionSites);
}

function same(left,right) {
  return Array.isArray(left) && left.length === right.length && left.every((value,index) => value === right[index]);
}

async function secureDirectory(directory) {
  await mkdir(directory,{recursive:true,mode:0o700});
  await chmod(directory,0o700);
  const handle = await open(directory,constants.O_RDONLY | constants.O_DIRECTORY | constants.O_NOFOLLOW);
  try {
    const metadata = await handle.stat();
    if (!metadata.isDirectory() || metadata.uid !== process.getuid() || (metadata.mode & 0o077) !== 0) fail();
  } finally { await handle.close(); }
}

async function journalKey(directory) {
  const filename = path.join(directory,'journal.key');
  let handle;
  try {
    handle = await open(filename,constants.O_WRONLY | constants.O_CREAT | constants.O_EXCL | constants.O_NOFOLLOW,0o600);
    const key = randomBytes(32);
    await handle.writeFile(key);
    await handle.sync();
    return key;
  } catch (error) {
    if (error?.code !== 'EEXIST') throw error;
  } finally { await handle?.close(); }
  handle = await open(filename,constants.O_RDONLY | constants.O_NOFOLLOW);
  try {
    const metadata = await handle.stat();
    if (!metadata.isFile() || metadata.uid !== process.getuid() || (metadata.mode & 0o077) !== 0
        || metadata.nlink !== 1 || metadata.size !== 32) fail();
    const key = Buffer.alloc(32);
    const {bytesRead} = await handle.read(key,0,key.length,0);
    if (bytesRead !== key.length) { key.fill(0); fail(); }
    return key;
  } finally { await handle.close(); }
}

async function writeOutcome(filename,requestId,receipt) {
  const handle = await open(filename,constants.O_WRONLY | constants.O_CREAT | constants.O_EXCL | constants.O_NOFOLLOW,0o600);
  try {
    await handle.writeFile(JSON.stringify({request_id:requestId,receipt}));
    await handle.sync();
  } finally { await handle.close(); }
}

function digest(value) { return createHash('sha256').update(value).digest('hex'); }
function text(value) {
  if (typeof value !== 'string' || !value || value.length > 512 || /[\x00-\x1f\x7f]/.test(value)) fail();
  return value;
}
function textList(value,empty) {
  if (!Array.isArray(value) || value.length > 32 || (!empty && value.length === 0)) fail();
  const result = value.map(text);
  if (new Set(result).size !== result.length) fail();
  return result;
}
function fail() {
  throw Object.assign(new Error('cookie_import_recovery_required'),{
    code:'cookie_import_recovery_required',recoveryRequired:true,
  });
}

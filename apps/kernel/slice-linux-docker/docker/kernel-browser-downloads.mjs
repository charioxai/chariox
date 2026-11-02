// MP-08/MP-10/MP-11: one CDP download spool, with completed files owned by a Room.
import { mkdir, lstat, rename, rm } from 'node:fs/promises';
import { createHash } from 'node:crypto';
import path from 'node:path';
import { userInfo } from 'node:os';
import { assertBrowserDownloadHeadroom } from './browser-controller-files.mjs';

const identity = value => typeof value === 'string' && /^[A-Za-z0-9_-]{1,128}$/.test(value);
const bytes = value => Number.isSafeInteger(value) && value >= 0 ? value : 0;
const sessionKey = session => createHash('sha256').update(session).digest('hex');
const failure = (code, message) => Object.assign(new Error(message), {code});

export class HostDownloads {
  constructor({ ownerRoot, notify, maxFileBytes = 256 * 1024 * 1024, maxSessionBytes = 1024 * 1024 * 1024 } = {}) {
    if (typeof ownerRoot !== 'string' || !ownerRoot) throw new Error('browser download owner root required');
    this.directory = path.join(userInfo().homedir, '.local/share/chariox/browser-downloads', sessionKey(ownerRoot));
    this.root = this.directory;
    this.notify = notify ?? (() => {});
    this.maxFileBytes = maxFileBytes;
    this.maxSessionBytes = maxSessionBytes;
    this.records = new Map();
    this.bindings = new Map();
    this.endedSessions = new Set();
    this.pending = new Set();
  }
  async start(connection, browser) {
    // Use the OS account's data directory, never cwd, TMPDIR or protected CHARIOX_HOME.
    this.root = this.directory;
    // A replacement controller retires the stopped browser's transient artifacts too.
    await rm(this.root, {recursive:true,force:true});
    await mkdir(this.root, {recursive:true,mode:0o700});
    this.spool = path.join(this.root, 'incoming');
    await mkdir(this.spool, {mode:0o700});
    await assertBrowserDownloadHeadroom({downloadDirectory:this.spool});
    this.connection = connection;
    this.browser = browser;
    browser.downloadDirectory = this.spool;
    this.off = connection.subscribe(message => this.observe(message));
    await connection.send('Browser.setDownloadBehavior', {behavior:'allowAndName',downloadPath:this.spool,eventsEnabled:true});
  }
  bind(tab, session) {
    if (typeof session === 'string' && session) this.bindings.set(tab.target_id, {tab_id:tab.tab_id,session});
  }
  track(promise) {
    this.pending.add(promise);
    promise.finally(() => this.pending.delete(promise)).catch(() => {});
    return promise;
  }
  async settle() { while (this.pending.size) await Promise.all([...this.pending]); }
  observe({method,params = {}}) {
    const guid = params.guid;
    if (!identity(guid)) return;
    if (method === 'Browser.downloadWillBegin') {
      if (this.records.has(guid)) return;
      const target = this.browser.targetsByFrame?.get(params.frameId);
      const binding = this.bindings.get(target) ?? {tab_id:null,session:"browser-viewer"};
      if (this.records.size >= 200) {
        this.track(this.connection.send('Browser.cancelDownload',{guid}).catch(() => {}));
        return;
      }
      const name = path.basename(String(params.suggestedFilename ?? 'download').replaceAll('\\','/')).slice(0,255) || 'download';
      const record = {download_id:guid,...binding,name,state:'in_progress',bytes:0,path:null,reserved:0,reason:null};
      this.records.set(guid, record);
      if (this.endedSessions.has(binding.session)) { record.reason = 'session_ended'; this.track(this.requestCancel(record)); }
      this.notify();
    }
    if (method !== 'Browser.downloadProgress') return;
    const record = this.records.get(guid);
    if (!record || record.state !== 'in_progress' || record.finishing) return;
    record.bytes = Math.max(record.bytes,bytes(params.receivedBytes));
    record.reserved = Math.max(record.bytes,bytes(params.totalBytes));
    const total = [...this.records.values()].filter(other => other.session === record.session && ['in_progress','done'].includes(other.state)).reduce((sum,other) => sum + other.reserved,0);
    const reason = record.reserved > this.maxFileBytes ? 'file_size_limit' : total > this.maxSessionBytes ? 'session_size_limit' : null;
    if (reason && !record.reason) {
      record.reason = reason;
      if (params.state === 'inProgress') this.track(this.requestCancel(record));
    }
    if (params.state === 'completed' || params.state === 'canceled') {
      record.finishing = true;
      this.track(this.finish(record,params.state));
    }
    this.notify();
  }
  async requestCancel(record) {
    try { await this.connection.send('Browser.cancelDownload',{guid:record.download_id}); }
    catch { record.reason = 'cancellation_failed'; this.notify(); }
  }
  async removeBytes(record) {
    await Promise.all(['','.crdownload'].map(suffix => rm(path.join(this.spool,record.download_id + suffix),{force:true})));
  }
  async finish(record, state) {
    try {
      if (state === 'completed' && !record.reason) {
        const source = path.join(this.spool,record.download_id), metadata = await lstat(source);
        if (!metadata.isFile() || metadata.size !== record.bytes) throw failure('download_file_invalid','download file is unavailable');
        const directory = path.join(this.root,sessionKey(record.session));
        await mkdir(directory,{recursive:true,mode:0o700});
        const destination = path.join(directory,record.download_id);
        await rename(source,destination);
        record.path = destination;
        record.state = 'done';
      } else {
        await this.removeBytes(record);
        record.state = record.reason === 'cancelled' ? 'cancelled' : 'failed';
        record.reason ??= 'interrupted';
      }
    } catch {
      record.state = 'failed'; record.reason = 'storage_unavailable'; record.path = null;
      await this.removeBytes(record);
    } finally { record.finishing = false; this.notify(); }
  }
  state() {
    return [...this.records.values()].filter(record => record.tab_id !== null).map(({download_id,tab_id,name,state,bytes,path,reason}) => ({download_id,tab_id,name,state,bytes,path,...(reason ? {reason} : {})}));
  }
  async cancel(guid, tab) {
    const record = this.records.get(guid);
    if (!record || record.tab_id !== tab) throw failure('browser_download_invalid','download does not belong to this tab; read state');
    if (record.state !== 'in_progress' || record.finishing) throw failure('browser_download_not_active','download is no longer active; read state');
    const previous = record.reason;
    record.reason ??= 'cancelled';
    try { await this.connection.send('Browser.cancelDownload',{guid}); }
    catch (error) { record.reason = previous; throw error; }
    return {download_id:guid,cancellation_requested:true};
  }
  async cleanupSession(session) {
    this.endedSessions.add(session);
    const records = [...this.records.values()].filter(record => record.session === session);
    for (const record of records) {
      if (record.state === 'in_progress' && !record.finishing) await this.connection.send('Browser.cancelDownload',{guid:record.download_id});
    }
    await this.settle();
    for (const record of records) { await this.removeBytes(record); this.records.delete(record.download_id); }
    await rm(path.join(this.root,sessionKey(session)),{recursive:true,force:true});
    this.notify();
  }
  async close() {
    this.off?.(); this.off = null;
    await this.settle();
    if (this.root) await rm(this.root,{recursive:true,force:true});
    this.root = null; this.records.clear(); this.bindings.clear(); this.endedSessions.clear();
  }
}

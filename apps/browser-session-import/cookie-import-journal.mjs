import {createCipheriv, createDecipheriv, createHash, randomBytes} from 'node:crypto';
import {constants} from 'node:fs';
import {lstat, open, unlink} from 'node:fs/promises';
import path from 'node:path';

const magic = Buffer.from('CHXCJ001');
const headerBytes = magic.length + 12 + 16;
const maximumBytes = 8 * 1024 * 1024;
const busyDirectories = new Set();

// Internal encrypted storage, not consent or a destination mutation endpoint.
// The kernel must provide a private Environment directory and a recoverable key.
export async function openCookieImportJournal(options) {
  let {directory,key,binding} = options ?? {};
  if (!['darwin','linux'].includes(process.platform) || typeof directory !== 'string'
      || !path.isAbsolute(directory) || !Buffer.isBuffer(key) || key.length !== 32) {
    fail('cookie_import_journal_configuration',false);
  }
  const fields = ['userId','roomId','environmentId'].map(field => binding?.[field]);
  if (fields.some(value => typeof value !== 'string' || !value || value.length > 512 || /[\u0000-\u001f]/.test(value))) {
    fail('cookie_import_journal_configuration',false);
  }
  const aad = Buffer.from(JSON.stringify(['chariox-cookie-import-journal-v1',...fields]));
  const ownedKey = Buffer.from(key);
  directory = path.resolve(directory);
  const filename = path.join(directory,'cookie-import.pending');
  let directoryHandle, directoryIdentity;
  try {
    directoryHandle = await open(directory,constants.O_RDONLY | constants.O_DIRECTORY | constants.O_NOFOLLOW);
    directoryIdentity = await directoryHandle.stat();
    if (!directoryIdentity.isDirectory() || directoryIdentity.uid !== process.getuid()
        || (directoryIdentity.mode & 0o077) !== 0) fail('cookie_import_journal_directory');
  } catch {
    ownedKey.fill(0);
    try { await directoryHandle?.close(); } catch { /* Preserve the fixed setup error. */ }
    fail('cookie_import_journal_directory');
  }
  let closed = false;
  let busy = false;
  let unavailable = false;
  const directoryId = `${directoryIdentity.dev}:${directoryIdentity.ino}`;
  const run = async operation => {
    if (closed) fail('cookie_import_journal_closed');
    if (unavailable) fail('cookie_import_journal_unavailable');
    if (busyDirectories.has(directoryId)) fail('cookie_import_journal_busy');
    busy = true;
    busyDirectories.add(directoryId);
    try {
      const current = await lstat(directory);
      if (!current.isDirectory() || current.dev !== directoryIdentity.dev || current.ino !== directoryIdentity.ino
          || current.uid !== process.getuid() || (current.mode & 0o077) !== 0) fail('cookie_import_journal_directory');
      return await operation();
    } catch (error) {
      if (error instanceof CookieImportJournalError) throw error;
      unavailable = true;
      fail('cookie_import_journal_io_failed');
    } finally { busy = false; busyDirectories.delete(directoryId); }
  };
  const read = async () => {
    let file;
    try { file = await open(filename,constants.O_RDONLY | constants.O_NOFOLLOW | constants.O_NONBLOCK); }
    catch (error) { if (error.code === 'ENOENT') return null; throw error; }
    let sealed, bytes;
    try {
      const metadata = await file.stat();
      if (!metadata.isFile() || metadata.uid !== process.getuid() || (metadata.mode & 0o077) !== 0
          || metadata.nlink !== 1 || metadata.size <= headerBytes || metadata.size > maximumBytes + headerBytes) {
        fail('cookie_import_journal_invalid');
      }
      sealed = Buffer.alloc(metadata.size + 1);
      let length = 0;
      while (length < sealed.length) {
        const {bytesRead} = await file.read(sealed,length,sealed.length - length,null);
        if (!bytesRead) break;
        length += bytesRead;
      }
      if (length !== metadata.size || !sealed.subarray(0,magic.length).equals(magic)) fail('cookie_import_journal_invalid');
      const record = sealed.subarray(0,length);
      const decipher = createDecipheriv('aes-256-gcm',ownedKey,record.subarray(8,20),{authTagLength:16});
      decipher.setAAD(aad);
      decipher.setAuthTag(record.subarray(20,36));
      let partial;
      try {
        partial = decipher.update(record.subarray(headerBytes));
        bytes = Buffer.concat([partial,decipher.final()]);
        return {bytes,receipt:digest(record)};
      } catch { fail('cookie_import_journal_invalid'); }
      finally { partial?.fill(0); }
    } finally {
      sealed?.fill(0);
      try { await file.close(); }
      catch (error) { bytes?.fill(0); throw error; }
    }
  };
  return {
    async prepare(bytes) {
      if (!Buffer.isBuffer(bytes) || !bytes.length || bytes.length > maximumBytes) {
        fail('cookie_import_journal_payload',false);
      }
      const plaintext = Buffer.from(bytes);
      try { return await run(async () => {
        const nonce = randomBytes(12);
        const cipher = createCipheriv('aes-256-gcm',ownedKey,nonce,{authTagLength:16});
        cipher.setAAD(aad);
        const ciphertext = Buffer.concat([cipher.update(plaintext),cipher.final()]);
        plaintext.fill(0);
        const sealed = Buffer.concat([magic,nonce,cipher.getAuthTag(),ciphertext]);
        let file;
        try {
          file = await open(filename,constants.O_WRONLY | constants.O_CREAT | constants.O_EXCL | constants.O_NOFOLLOW,0o600);
        } catch (error) {
          if (error.code === 'EEXIST') fail('cookie_import_recovery_required');
          throw error;
        }
        try {
          await file.writeFile(sealed);
          await file.sync();
          await directoryHandle.sync();
          return digest(sealed);
        } finally { await file.close(); }
      }); } finally { plaintext.fill(0); }
    },
    read:() => run(read),
    // The caller must verify browser recovery or durably record the outcome first.
    discard:receipt => run(async () => {
      if (typeof receipt !== 'string' || !/^[a-f0-9]{64}$/.test(receipt)) fail('cookie_import_journal_receipt');
      const pending = await read();
      if (!pending) fail('cookie_import_journal_receipt');
      pending.bytes.fill(0);
      if (pending.receipt !== receipt) fail('cookie_import_journal_receipt');
      await unlink(filename);
      await directoryHandle.sync();
    }),
    async close() {
      if (busy) fail('cookie_import_journal_busy');
      if (closed) return;
      closed = true;
      ownedKey.fill(0);
      try { await directoryHandle.close(); }
      catch { fail('cookie_import_journal_io_failed'); }
    },
  };
}

function digest(bytes) { return createHash('sha256').update(bytes).digest('hex'); }
class CookieImportJournalError extends Error {
  constructor(code,recoveryRequired) { super(code); this.code = code; this.recoveryRequired = recoveryRequired; }
}
function fail(code,recoveryRequired = true) { throw new CookieImportJournalError(code,recoveryRequired); }

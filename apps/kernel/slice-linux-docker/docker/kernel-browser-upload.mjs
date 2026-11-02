// MP-08/MP-09/MP-10/MP-11: workspace paths use the existing upload stager and input authority.
import path from 'node:path';
import { realpath } from 'node:fs/promises';
import { uploadBrowserFiles, BrowserFileTransferError } from './browser-controller-files.mjs';
import { assertCurrentDocument, assertNotCancelled } from './browser-controller-actions.mjs';
import { UserDomainRefusal } from './kernel-browser-refusal.mjs';

const within = (root, file) => { const relative = path.relative(root, file); return !path.isAbsolute(relative) && relative !== '..' && !relative.startsWith('..' + path.sep); };
const invalid = message => new BrowserFileTransferError('browser_upload_invalid', message);
const publicReasons = new Set([
  'supply path or paths, not both', 'upload requires 1 through 20 files',
  'upload requires a snapshot node_ref as target', 'Room workspace is unavailable',
  'upload requires bounded file paths', 'upload file is missing or unavailable',
  'browser uploads require regular files with a bounded size',
  'browser upload exceeds 536870912 total bytes',
  'upload target must be an input type=file; drop zones and chooser buttons are unsupported',
  'upload target must allow multiple files',
]);

export function publicUploadFailure(error) {
  if (!(error instanceof BrowserFileTransferError)) return null;
  if (error.code === 'browser_upload_invalid' && publicReasons.has(error.message)) return { code: error.code, message: error.message };
  if (error.code === 'browser_upload_staging_unavailable') return { code: error.code, message: 'upload staging is unavailable; check host storage and retry with a fresh snapshot' };
  return null;
}

export async function uploadHostFiles(browser, tab, input, { uploadRoots, signal, inputGuard, onDispatch, stageUploads } = {}) {
  assertNotCancelled(signal);
  if (!Array.isArray(uploadRoots) || uploadRoots.length !== 1) throw new UserDomainRefusal('not_granted');
  const candidates = input.path !== undefined ? [input.path] : input.paths;
  if (input.path !== undefined && input.paths !== undefined) throw invalid('supply path or paths, not both');
  if (!Array.isArray(candidates) || candidates.length < 1 || candidates.length > 20) throw invalid('upload requires 1 through 20 files');
  if (typeof input.target !== 'string' || !/^backend:[1-9][0-9]{0,15}$/.test(input.target)) throw invalid('upload requires a snapshot node_ref as target');
  let root;
  try { root = await realpath(uploadRoots[0]); }
  catch { throw invalid('Room workspace is unavailable'); }
  const files = [];
  for (const candidate of candidates) {
    if (typeof candidate !== 'string' || !candidate || Buffer.byteLength(candidate) > 4096 || candidate.includes('\0')) throw invalid('upload requires bounded file paths');
    if (candidate.split(path.sep).includes('..')) throw new UserDomainRefusal('not_granted');
    const absolute = path.resolve(root, candidate);
    if (!within(root, absolute)) throw new UserDomainRefusal('not_granted');
    let resolved;
    try { resolved = await realpath(absolute); }
    catch { throw invalid('upload file is missing or unavailable'); }
    if (!within(root, resolved)) throw new UserDomainRefusal('not_granted');
    files.push(resolved);
  }
  const { connection, sessionId } = await browser.resolvePageTarget(tab.target_id);
  const check = async () => {
    assertNotCancelled(signal);
    await inputGuard?.();
    await assertCurrentDocument(connection, sessionId, tab.target_id, tab.document_id);
    assertNotCancelled(signal);
  };
  try {
    return await uploadBrowserFiles({ connection, sessionId, targetId: tab.target_id, documentId: tab.document_id,
      nodeRef: input.target, filePaths: files, uploadRoots: [root], signal, stageUploads,
      fileInputOnly: true, assertContext: check,
      withInput: async operation => {
        await check();
        const end = onDispatch?.();
        try { return await operation(); } finally { end?.(); }
      },
    });
  } catch (error) {
    if (error?.code === 'browser_upload_denied') throw new UserDomainRefusal('not_granted');
    throw error;
  }
}

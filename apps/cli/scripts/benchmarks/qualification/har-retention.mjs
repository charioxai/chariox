// MP-08 / MP-10 / MP-11 H13: immutable final flush copy and verified hash.
import { open, realpath, readFile, unlink } from 'node:fs/promises'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { sha256 } from './admission.mjs'
const repository = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../../../../..')

export async function retainFinalHar(snapshot, filename) {
  const capture = snapshot?.log?._capture
  if (capture?.flushAcknowledged !== true || capture.activeRequests !== 0 || capture.missingExtraInfo !== 0
    || capture.unmatchedExtraInfo !== 0 || capture.errors?.length || !snapshot.log.entries.length) {
    throw new Error('MP-10 HAR final flush incomplete')
  }
  if (!path.isAbsolute(filename)) throw new Error('MP-10 external absolute evidence path required')
  const parent = await realpath(path.dirname(filename))
  const sourceRoot = await realpath(repository)
  const relative = path.relative(sourceRoot, parent)
  if (!relative || (!relative.startsWith(`..${path.sep}`) && relative !== '..' && !path.isAbsolute(relative))) {
    throw new Error('MP-10 evidence must remain outside checkout')
  }
  const target = path.join(parent, path.basename(filename))
  const bytes = Buffer.from(JSON.stringify(snapshot) + '\n')
  const file = await open(target, 'wx', 0o600)
  let inode
  try {
    inode = await file.stat()
    await file.writeFile(bytes); await file.sync()
    const copied = await readFile(target)
    if (sha256(copied) !== sha256(bytes)) throw new Error('MP-10 copied HAR hash mismatch')
    return { mpItems: ['MP-08','MP-10','MP-11'], file: target, bytes: copied.length,
      sha256: sha256(copied), flushAcknowledged: true, metadataOnly: true,
      entries: snapshot.log.entries.length, scoredCampaign: false }
  } catch (error) {
    // Remove only the exact file this invocation created; never overwrite an old HAR.
    const current = await open(target, 'r').catch(() => null)
    if (current) { const st = await current.stat(); await current.close(); if (st.dev === inode?.dev && st.ino === inode?.ino) await unlink(target) }
    throw error
  } finally { await file.close() }
}

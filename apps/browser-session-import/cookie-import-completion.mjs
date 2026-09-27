// Internal completion boundary. Call only after verified import or recovery,
// while retaining the kernel's exclusive Environment execution guard.
export async function completeCookieImport({receipt,journal,recordOutcome,clearPending}) {
  if (typeof receipt !== 'string' || !/^[a-f0-9]{64}$/.test(receipt)
      || typeof journal?.discard !== 'function'
      || typeof recordOutcome !== 'function' || typeof clearPending !== 'function') {
    throw failure();
  }
  try {
    // Each callback must resolve only after its durable write is acknowledged.
    await recordOutcome();
    await journal.discard(receipt);
    await clearPending();
  } catch {
    // Never expose callback errors, which may contain storage or credential data.
    // An uncertain acknowledgement must leave the Environment quarantined.
    throw failure();
  }
}

// Restart-only cleanup. The kernel must confirm the matching request's durable
// completed state, not merely the existence of an import row. A missing journal
// alone never proves success. Keep exclusive Environment ownership throughout.
export async function resumeCookieImportCleanup({receipt,journal,confirmOutcome,clearPending}) {
  if (typeof journal?.read !== 'function' || typeof journal?.discard !== 'function'
      || typeof confirmOutcome !== 'function' || typeof clearPending !== 'function') throw failure();
  try {
    if (await confirmOutcome() !== true) throw failure();
    const pending = await journal.read();
    if (pending) {
      pending.bytes.fill(0);
      if (typeof receipt !== 'string' || !/^[a-f0-9]{64}$/.test(receipt)
          || pending.receipt !== receipt) throw failure();
      await journal.discard(receipt);
    }
    await clearPending();
  } catch { throw failure(); }
}

function failure() {
  return Object.assign(new Error('cookie_import_completion_failed'), {
    code:'cookie_import_completion_failed',recoveryRequired:true,
  });
}

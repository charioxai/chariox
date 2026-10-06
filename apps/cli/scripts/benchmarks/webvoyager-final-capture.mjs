// MP-08/MP-10: passive evidence retry only; no solver, navigation or judge replay.
export async function captureFinalEvidence({ capture, pause = ms => new Promise(resolve => setTimeout(resolve, ms)) }) {
  for (let attempt = 1; attempt <= 3; attempt++) {
    try { await capture(); return { attempts: attempt } }
    catch (error) { if (attempt === 3) throw error; await pause(1000) }
  }
}

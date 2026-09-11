let began
let state

export function resetCancellationFixture() {
  state = {started:false,mutations:0,rolledBack:false,cleanupComplete:false}
  began = Promise.withResolvers()
}

export function cancellationFixtureState() { return structuredClone(state) }
export function waitForCancellationFixture() { return began.promise }

export async function applyProductionBrowserImport({signal,params}) {
  state.started = true
  began.resolve()
  await Promise.race([
    new Promise(resolve => signal?.addEventListener("abort",resolve,{once:true})),
    new Promise(resolve => setTimeout(resolve,100)),
  ])
  if (!signal?.aborted || params.ignore_abort === true) {
    state.mutations += 1
    state.cleanupComplete = true
    return {status:"applied",results:params.domains.map(domain => ({domain,status:"imported",cookie_count:1}))}
  }
  await new Promise(resolve => setImmediate(resolve))
  state.rolledBack = true
  state.cleanupComplete = true
  return {status:"rolled_back",results:[]}
}

export async function recoverProductionBrowserImport() { return {status:"verified"} }

resetCancellationFixture()

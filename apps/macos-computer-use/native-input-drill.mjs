// Run with the coordinator's admitted agent/owner transport and exact public
// fixture. Hooks do not bootstrap credentials or manufacture an acceptance PASS.
export async function runNativeInputDrill({ agentInput, ownerRequest, binding, click,
  selection, expectedSelection, physicalHuman, fatalPress, stop }) {
  const receipt = { status: 'FAIL', checks: [] };
  const check = (name, ok) => {
    receipt.checks.push({ name, passed: !!ok });
    if (!ok) throw new Error(name);
  };
  const refused = async (dispatch, expected) => {
    try { await dispatch(); return false; }
    catch (error) { return expected(error); }
  };
  const input = command => agentInput({ op: 'input', ...binding, input: command });
  try {
    await input(click);
    check('native-click-independent-selection', await selection() === expectedSelection);
    const before = await selection();
    const human = await physicalHuman();
    check('foreign-event-observed-and-physical-owner-projected', human?.observed === true && human?.actor_id === 'human:physical-owner');
    check('agent-paused-after-physical-human', await refused(() => input(click), error =>
      ['user_domain_not_granted', 'unsupported', 'browser_action_cancelled'].includes(error?.code)));
    check('paused-agent-made-no-mutation', await selection() === before);
    await ownerRequest({ op: 'release', ...binding });
    await input(click);
    check('explicit-owner-release-resumes', await selection() === expectedSelection);
    // This hook must observe a real public-fixture keyDown while the bounded
    // 100 ms Action is pending, then kill only the pinned paired helper. It
    // returns the kernel's actual failure and independent fixture receipt.
    const fatal = await fatalPress(() => input({ kind: 'hold', key: 'Right', duration_ms: 100 }));
    check('fatal-owned-press-observed', fatal?.down_observed === true && fatal?.helper_identity_verified === true);
    check('fatal-input-requires-owner-reset', fatal?.error?.message?.includes('owner-reset-required'));
    check('stop-start-cannot-clear-fatal-press', await refused(() => ownerRequest({ op: 'start' }),
      error => error?.message?.includes('owner-reset-required')));
    receipt.status = 'PASS';
    return receipt;
  } catch {
    const error = new Error('native input drill failed');
    error.receipt = receipt;
    throw error;
  } finally {
    try { await stop(); }
    catch {
      receipt.status = 'FAIL'; receipt.checks.push({ name: 'owned-seat-cleanup', passed: false });
      const error = new Error('native input drill cleanup failed'); error.receipt = receipt; throw error;
    }
  }
}

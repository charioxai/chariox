// MP-08 / MP-10 / MP-11: real TUI first prompt, official Codex, hosted WSS.
import { stack, sleep, variant } from './context-switch/runtime.mjs';
import { shapedRemoteTui } from './context-switch/remote-tui.mjs';

const label = process.env.UIQA_BUILD_LABEL ?? 'candidate';
const expectUnverified = process.env.UIQA_EXPECT_UNVERIFIED === '1';
const source = process.env.UIQA_CODEX_PROFILE;
if (!source) throw Error('UIQA_CODEX_PROFILE must name an existing product-linked Codex scope');
let run, remote;
const result = { MP: ['MP-08', 'MP-10', 'MP-11'], status: 'FAIL', expectUnverified };
try {
  run = await stack(label, process.env.UIQA_RUN_NAME ?? `unknown-account-${label}`, 'unused-claude-validation-placeholder');
  const profile = variant(await run.client.send({ LinkProviderAccountProfile: {
    provider: 'codex', label: 'fresh-linked-account', path: source,
  } }), 'ProviderAccountProfile').profile;
  if (profile.auth_state !== 'unknown') throw Error('Link must start with an unknown observation');
  run.profiles.codex = profile.profile_id;
  // Setup uses the public kernel API; the action under test uses the real TUI.
  const created = variant(await run.client.send({ CreateSession: {
    workspace_id: run.tree, worktree_id: run.tree, alias: 'unknown-first-prompt',
    agent_defaults: { provider: 'codex', account_profile: profile.profile_id,
      model: 'gpt-6-luna', effort: 'low', permission_level: 'required' },
  } }), 'SessionCreated');
  if (process.env.CTXVAL_ORIGIN) remote = await shapedRemoteTui(run, { useTui: true });
  await run.launch('codex', 'gpt-6-luna', created.session.id);
  const account = async () => variant(await run.client.send({ GetProviderAccountProfile: {
    provider: 'codex', account_profile: profile.profile_id,
  } }), 'ProviderAccountProfile').profile;
  const before = await account();
  await run.record('before-first-prompt', { authState: before.auth_state });
  if (before.auth_state !== 'unknown') throw Error('A pre-prompt path refreshed the profile; this run cannot prove first-use checking');
  const first = await run.tui.send('submit_prompt', { prompt: 'Without tools, reply exactly UIQA_FIRST_OK.' });
  await sleep(500);
  await run.capture('first-prompt');
  if (expectUnverified) {
    if (!/not been verified|refresh.*Provider Accounts/i.test(first.footer?.message ?? '')) {
      throw Error('Base did not show the expected account-unverified refusal');
    }
    if ((await account()).auth_state !== 'unknown') throw Error('Base unexpectedly checked authentication');
    result.status = 'EXPECTED RED';
    result.firstSeam = first.footer.message;
  } else {
    if (first.footer?.tone === 'error') throw Error('First prompt refused: ' + first.footer.message);
    await run.turn('', 'UIQA_FIRST_OK', 180000, false);
    const observed = await account();
    if (observed.auth_state !== 'authenticated') throw Error('First prompt did not authenticate the selected profile');
    if (!Number.isInteger(observed.last_validated_at_ms)) throw Error('Missing native authentication observation timestamp');
    result.firstCheckedAt = observed.last_validated_at_ms;
    await sleep(10);
    await run.turn('Without tools, reply briefly.', 'UIQA_SECOND_OK', 180000);
    const second = await account();
    if (second.last_validated_at_ms !== observed.last_validated_at_ms) throw Error('Second prompt rechecked the known profile');
    result.secondCheckedAt = second.last_validated_at_ms;
    result.binding = await run.binding();
    result.status = 'PASS';
  }
} catch (error) {
  result.firstSeam = String(error?.message ?? error);
  if (!run) console.error(result.firstSeam.replace(/https?:\/\/\S+/g, '[URL]').slice(0, 1000));
  if (run?.tui) await run.capture('failure').catch(() => {});
} finally {
  if (run) await run.record('result', result);
  await run?.close();
  await remote?.close();
}
if (!['PASS', 'EXPECTED RED'].includes(result.status)) process.exitCode = 1;

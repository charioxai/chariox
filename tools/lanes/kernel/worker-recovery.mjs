// MP-08 / MP-10 / MP-11: inspect kernel-owned worker/provider recovery, never launch receipt replay.
export async function inspectWorker(rpc, run) {
  const response = await rpc({ GetSessionState: { session_id: run.launch.sessionId } });
  const session = response.SessionState.session;
  const agent = (response.SessionState.agents || session.agents || []).find(agent => agent.id === run.launch.agentId);
  if (!agent) throw new Error(`worker missing after restart: ${run.key}`);
  const activity = (response.SessionState.agent_activity || session.agent_activity)?.[agent.id];
  const runId = activity?.active_turn?.provider_run_id;
  const providerRun = runId ? (await rpc({ GetProviderRun: { provider_run_id: runId } })).ProviderRun.provider_run : undefined;
  const completed = activity?.last_completed_turn;
  // The account the kernel launched this worker on: the run's profile, else the agent's own, shown by its label.
  const profileId = providerRun?.account_profile ?? agent.account_profile;
  const labels = profileId && agent.provider === 'claude' ? (await rpc({ ListProviderAccountProfiles: { provider: 'claude' } })).ProviderAccountProfilesListed.profiles : [];
  const account = labels.find(profile => profile.profile_id === profileId)?.label;
  return { provider: agent.provider, account, permission: agent.permission_level_override || session.permission_level,
    state: activity?.status,
    providerActivityAtMs: providerRun?.state === 'Running' ? providerRun.last_activity_at_ms : undefined,
    completedAtMs: completed?.completed_at_ms,
    completedSuccessfully: completed?.settlement_status === 'completed' && !completed?.provider_termination };
}

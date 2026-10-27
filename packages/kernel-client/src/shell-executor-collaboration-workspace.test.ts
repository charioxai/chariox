import assert from "node:assert/strict"
import { mkdtemp, readFile, rm } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"
import test from "node:test"

import type {
  AgentInstance,
  CharioxMcpServerConfig,
  CharioxSkillMetadata,
  ProviderProcessInfo,
  WorkspaceLinkDefinition,
} from "./kernel-types.js"
import { applyShellCommandResult, createDefaultShellContext, parseShellCommand } from "./shell-core.js"
import { executeShellCommand } from "./shell-executor.js"
import {
  daemonHealth,
  fakeClient,
  makeAgent,
  makeSession,
  makeWorkflow,
  makeWorkflowPublication,
  makeWorkflowRun,
  makeWorkflowWatchdog,
} from "./shell-executor.test-support.js"

test("executeShellCommand attaches standalone shell clients when switching sessions", async () => {
  const session = makeSession({ id: "session-2", worktree_id: "/repo/qa", focused_agent_id: "agent-1" })
  const requests: Record<string, unknown>[] = []
  const fake = {
    client: {
      send: async (request: Record<string, unknown>) => {
        requests.push(request)
        if ("CreateSession" in request) {
          return { SessionCreated: { session } }
        }
        return { SessionAttached: { attachment: { id: "attachment-shell" } } }
      },
    },
  }
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo" })
  const result = await executeShellCommand(parseShellCommand("session new --dir qa as s"), context, {
    client: fake.client,
    clientId: "chariox-shell-test",
  })
  assert.equal(result.ok, true)
  assert.equal(result.contextUpdates?.attachmentId, "attachment-shell")
  assert.deepEqual(requests, [
    { CreateSession: { workspace_id: "/repo", worktree_id: "/repo/qa", alias: null, slice_ref: null } },
    { AttachToSession: { session_id: "session-2", client_id: "chariox-shell-test", capability_level: "FullTerminal" } },
  ])
})

test("executeShellCommand manages session invites and members", async () => {
  const session = makeSession({
    id: "session-1",
    owner_user_id: "local",
    members: [{ user_id: "local", joined_at_ms: 0, invited_by_user_id: null }],
    invites: [],
  })
  const invite = {
    invite_id: "invite-1",
    session_id: "session-1",
    created_by_user_id: "local",
    created_at_ms: 100,
    expires_at_ms: null,
    max_uses: 1,
    used_count: 0,
    revoked_at_ms: null,
  }
  const requests: Record<string, unknown>[] = []
  const fake = {
    client: {
      send: async (request: Record<string, unknown>) => {
        requests.push(request)
        if ("CreateSessionInvite" in request) {
          return { SessionInviteCreated: { invite: { invite, invite_token: "chariox-session-invite-v1.token" }, session } }
        }
        if ("JoinSessionInvite" in request) {
          return {
            SessionInviteJoined: {
              member: { user_id: "ana", joined_at_ms: 200, invited_by_user_id: "local" },
              session: { ...session, members: [...(session.members ?? []), { user_id: "ana", joined_at_ms: 200, invited_by_user_id: "local" }] },
            },
          }
        }
        if ("ListSessionMembers" in request) {
          return {
            SessionMembersListed: {
              members: session.members,
              invites: [
                invite,
                { ...invite, invite_id: "invite-revoked", revoked_at_ms: 300 },
                { ...invite, invite_id: "invite-expired", expires_at_ms: 1 },
                { ...invite, invite_id: "invite-exhausted", used_count: 1 },
              ],
            },
          }
        }
        if ("RevokeSessionInvite" in request) {
          return { SessionInviteRevoked: { invite: { ...invite, revoked_at_ms: 300 }, session } }
        }
        return { SessionAttached: { attachment: { id: "attachment-shell" } } }
      },
    },
  }
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo", sessionId: "session-1" })
  const inviteResult = await executeShellCommand(parseShellCommand("session invite create"), context, { client: fake.client })
  const joinResult = await executeShellCommand(parseShellCommand("session join chariox-session-invite-v1.token ana"), context, { client: fake.client, clientId: "shell-ana" })
  const membersResult = await executeShellCommand(parseShellCommand("session members"), context, { client: fake.client })
  const invitesResult = await executeShellCommand(parseShellCommand("session invites"), context, { client: fake.client })
  const revokeResult = await executeShellCommand(parseShellCommand("session revoke-invite invite-1"), context, { client: fake.client })

  assert.match(inviteResult.message ?? "", /session invite invite-1/)
  assert.match(joinResult.message ?? "", /joined session session-1 as ana/)
  assert.match(membersResult.message ?? "", /Session members/)
  assert.match(invitesResult.message ?? "", /Session invites\n- invite-1 uses=0\/1/)
  assert.doesNotMatch(invitesResult.message ?? "", /invite-(?:revoked|expired|exhausted)/)
  assert.match(revokeResult.message ?? "", /revoked session invite invite-1/)
  assert.deepEqual(requests, [
    { CreateSessionInvite: { session_id: "session-1", expires_in_ms: null, max_uses: 1, collaboration_level: "private" } },
    { JoinSessionInvite: { invite_token: "chariox-session-invite-v1.token", user_id: "ana" } },
    { AttachToSession: { session_id: "session-1", client_id: "shell-ana", capability_level: "FullTerminal" } },
    { ListSessionMembers: { session_id: "session-1" } },
    { ListSessionMembers: { session_id: "session-1" } },
    { RevokeSessionInvite: { session_id: "session-1", invite_ref: "invite-1" } },
  ])
})


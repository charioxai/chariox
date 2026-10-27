import {
  assert,
  createDefaultShellContext,
  executeShellCommand,
  fakeClient,
  makeAgent,
  makeSession,
  parseShellCommand,
  test,
} from "../shell-executor-agents-remote.test-support.js"
import type { AgentInstance } from "../shell-executor-agents-remote.test-support.js"

test("executeShellCommand rejects agent commands without current session", async () => {
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo" })
  const result = await executeShellCommand(parseShellCommand("agent list"), context, { client: fakeClient(() => ({})).client })
  assert.equal(result.ok, false)
  assert.match(result.message ?? "", /no current session/)
})

test("executeShellCommand lists remote machines", async () => {
  const fake = fakeClient((request) => {
    assert.deepEqual(request, { ListRemoteMachines: null })
    return {
      RemoteMachinesListed: {
        machines: [{
          machine_id: "machine-1",
          machine_alias: "mini",
          registry_alias: null,
          display_name: "mini",
          trust_status: "approved",
          online: true,
          pending: false,
          kernel_count: 1,
          available_providers: ["codex"],
        }, {
          machine_id: "machine-2",
          machine_alias: "cold",
          registry_alias: null,
          display_name: "cold",
          trust_status: "pending",
          online: true,
          pending: true,
          kernel_count: 0,
          available_providers: [],
        }],
      },
    }
  })
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo" })
  const result = await executeShellCommand(parseShellCommand("machine list"), context, { client: fake.client })
  assert.equal(result.ok, true)
  assert.match(result.message ?? "", /mini id=machine-1/)
  assert.match(result.message ?? "", /cold id=machine-2 status=pending/)
  assert.match(result.message ?? "", /next: approve with machine approve machine-2/)
})

test("executeShellCommand lists remote kernels with recovery hints", async () => {
  const fake = fakeClient((request) => {
    assert.deepEqual(request, { ListRemoteMachineKernels: { machine_ref: "machine-1" } })
    return {
      RemoteMachineKernelsListed: {
        kernels: [{
          kernel_id: "kernel-1",
          machine_id: "machine-1",
          machine_alias: "mini",
          relay_alias: "mini-kernel",
          kernel_alias: null,
          accepting_remote_leases: false,
          leased_agent_count: 0,
          local_session_count: 1,
          available_providers: [],
        }],
      },
    }
  })
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo" })
  const result = await executeShellCommand(parseShellCommand("machine kernels machine-1"), context, { client: fake.client })
  assert.equal(result.ok, true)
  assert.match(result.message ?? "", /machine machine-1 worker readiness: 0\/1 ready, 1 blocked/)
  assert.match(result.message ?? "", /mini-kernel id=kernel-1/)
  assert.match(result.message ?? "", /readiness=blocked/)
  assert.match(result.message ?? "", /accepting_remote_leases=false/)
  assert.match(result.message ?? "", /next: run \/machine kernels mini; enable remote leases on mini-kernel or choose another worker/)
})

test("executeShellCommand renders unknown remote lease state without a false recovery hint", async () => {
  const fake = fakeClient((request) => {
    assert.deepEqual(request, { ListRemoteMachineKernels: { machine_ref: "machine-1" } })
    return {
      RemoteMachineKernelsListed: {
        kernels: [{
          kernel_id: "kernel-1",
          machine_id: "machine-1",
          machine_alias: "mini",
          relay_alias: "mini-kernel",
          kernel_alias: null,
          leased_agent_count: 0,
          local_session_count: 1,
          available_providers: ["codex"],
        }],
      },
    }
  })
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo" })
  const result = await executeShellCommand(parseShellCommand("machine kernels machine-1"), context, { client: fake.client })

  assert.equal(result.ok, true)
  assert.match(result.message ?? "", /machine machine-1 worker readiness: 0\/1 ready, 1 unknown/)
  assert.match(result.message ?? "", /readiness=unknown/)
  assert.match(result.message ?? "", /accepting_remote_leases=unknown/)
  assert.doesNotMatch(result.message ?? "", /enable remote leases/)
})

test("executeShellCommand manages remote machine trust", async () => {
  const machine = {
    machine_id: "machine-1",
    machine_alias: "mini",
    registry_alias: "mini",
    display_name: "mini",
    trust_status: "approved",
    online: true,
    pending: false,
    kernel_count: 1,
    available_providers: ["codex"],
  }
  const requests: Record<string, unknown>[] = []
  const fake = {
    client: {
      send: async (request: Record<string, unknown>) => {
        requests.push(request)
        if ("ApproveRemoteMachine" in request) {
          return { RemoteMachineApproved: { machine } }
        }
        if ("RenameRemoteMachine" in request) {
          return { RemoteMachineRenamed: { machine: { ...machine, registry_alias: "builder" } } }
        }
        if ("ForgetRemoteMachine" in request) {
          return { RemoteMachineForgotten: { machine: { ...machine, trust_status: "forgotten" } } }
        }
        return {}
      },
    },
  }
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo" })

  const approveResult = await executeShellCommand(parseShellCommand("machine approve machine-1"), context, { client: fake.client })
  const renameResult = await executeShellCommand(parseShellCommand("machine rename machine-1 builder"), context, { client: fake.client })
  const revokeResult = await executeShellCommand(parseShellCommand("machine revoke machine-1"), context, { client: fake.client })

  assert.equal(approveResult.ok, true)
  assert.match(approveResult.message ?? "", /approved machine mini/)
  assert.equal(renameResult.ok, true)
  assert.match(renameResult.message ?? "", /renamed machine mini/)
  assert.equal(revokeResult.ok, true)
  assert.match(revokeResult.message ?? "", /revoked machine mini/)
  assert.deepEqual(requests, [
    { ApproveRemoteMachine: { machine_ref: "machine-1" } },
    { RenameRemoteMachine: { machine_ref: "machine-1", alias: "builder" } },
    { ForgetRemoteMachine: { machine_ref: "machine-1" } },
  ])
})

test("executeShellCommand creates and joins machine invites", async () => {
  const requests: Record<string, unknown>[] = []
  const fake = {
    client: {
      send: async (request: Record<string, unknown>) => {
        requests.push(request)
        if ("CreatePairingInvite" in request) {
          return {
            PairingInviteCreated: {
              invite: {
                intent: "machine",
                invite_id: "invite-1",
                invite_token: "chariox-invite-v1.machine",
                relay_url: "ws://relay",
                target_daemon_id: "daemon-1",
                target_daemon_alias: null,
                issued_at_ms: 1,
                expires_at_ms: 2,
              },
            },
          }
        }
        if ("JoinPairingInvite" in request) {
          return {
            PairingInviteJoined: {
              pairing: {
                intent: "machine",
                subject_id: "machine-2",
                relay_url: "ws://relay",
                target_daemon_id: "daemon-1",
                alias: "worker",
                public_key_thumbprint: "thumbprint-2",
                paired_at_ms: 3,
              },
            },
          }
        }
        return {}
      },
    },
  }
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo" })

  const inviteResult = await executeShellCommand(parseShellCommand("machine invite create worker"), context, { client: fake.client })
  const joinResult = await executeShellCommand(parseShellCommand("machine join chariox-invite-v1.machine machine-2 worker"), context, { client: fake.client })

  assert.equal(inviteResult.ok, true)
  assert.match(inviteResult.message ?? "", /machine invite invite-1/)
  assert.match(inviteResult.message ?? "", /token=chariox-invite-v1\.machine/)
  assert.equal(joinResult.ok, true)
  assert.match(joinResult.message ?? "", /joined machine machine-2 alias=worker/)
  assert.deepEqual(requests, [
    { CreatePairingInvite: { intent: "machine", alias: "worker", expires_in_ms: null } },
    {
      JoinPairingInvite: {
        invite_token: "chariox-invite-v1.machine",
        subject_id: "machine-2",
        public_key_thumbprint: null,
        alias: "worker",
      },
    },
  ])
})

test("executeShellCommand manages paired clients", async () => {
  const requests: Record<string, unknown>[] = []
  const fake = {
    client: {
      send: async (request: Record<string, unknown>) => {
        requests.push(request)
        if ("ListPairedClients" in request) {
          return {
            PairedClientsListed: {
              clients: [{
                client_id: "client-1",
                alias: "desk",
                public_key_thumbprint: "thumbprint-1",
                paired_at_ms: 42,
                revoked: false,
              }],
            },
          }
        }
        if ("RecordPairedClient" in request) {
          return {
            PairedClientRecorded: {
              client: {
                client_id: "client-2",
                alias: "laptop",
                public_key_thumbprint: "thumbprint-2",
                paired_at_ms: 84,
                revoked: false,
              },
            },
          }
        }
        if ("RevokePairedClient" in request) {
          return {
            PairedClientRevoked: {
              client: {
                client_id: "client-2",
                alias: "laptop",
                public_key_thumbprint: "thumbprint-2",
                paired_at_ms: 84,
                revoked: true,
              },
            },
          }
        }
        return {}
      },
    },
  }
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo" })

  const listResult = await executeShellCommand(parseShellCommand("client list"), context, { client: fake.client })
  const recordResult = await executeShellCommand(parseShellCommand("client record client-2 thumbprint-2 laptop"), context, { client: fake.client })
  const revokeResult = await executeShellCommand(parseShellCommand("client revoke client-2"), context, { client: fake.client })

  assert.equal(listResult.ok, true)
  assert.match(listResult.message ?? "", /desk id=client-1 thumbprint=thumbprint-1 paired_at_ms=42/)
  assert.equal(recordResult.ok, true)
  assert.match(recordResult.message ?? "", /paired client laptop id=client-2/)
  assert.equal(revokeResult.ok, true)
  assert.match(revokeResult.message ?? "", /revoked client laptop id=client-2/)
  assert.deepEqual(requests, [
    { ListPairedClients: null },
    {
      RecordPairedClient: {
        client_id: "client-2",
        public_key_thumbprint: "thumbprint-2",
        alias: "laptop",
        paired_at_ms: null,
      },
    },
    { RevokePairedClient: { client_id: "client-2" } },
  ])
})

test("executeShellCommand creates and joins client invites", async () => {
  const requests: Record<string, unknown>[] = []
  const fake = {
    client: {
      send: async (request: Record<string, unknown>) => {
        requests.push(request)
        if ("CreatePairingInvite" in request) {
          return {
            PairingInviteCreated: {
              invite: {
                intent: "client",
                invite_id: "invite-client",
                invite_token: "chariox-invite-v1.client",
                relay_url: "ws://relay",
                target_daemon_id: "daemon-1",
                target_daemon_alias: "home",
                issued_at_ms: 1,
                expires_at_ms: 2,
              },
            },
          }
        }
        if ("JoinPairingInvite" in request) {
          return {
            PairingInviteJoined: {
              pairing: {
                intent: "client",
                subject_id: "client-2",
                relay_url: "ws://relay",
                target_daemon_id: "daemon-1",
                alias: "desk",
                public_key_thumbprint: "thumbprint-client",
                paired_at_ms: 3,
              },
            },
          }
        }
        return {}
      },
    },
  }
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo" })

  const inviteResult = await executeShellCommand(parseShellCommand("client invite create desk"), context, { client: fake.client })
  const joinResult = await executeShellCommand(parseShellCommand("client join chariox-invite-v1.client client-2 desk"), context, { client: fake.client })

  assert.equal(inviteResult.ok, true)
  assert.match(inviteResult.message ?? "", /client invite invite-client/)
  assert.equal(joinResult.ok, true)
  assert.match(joinResult.message ?? "", /joined client client-2 alias=desk/)
  assert.deepEqual(requests, [
    { CreatePairingInvite: { intent: "client", alias: "desk", expires_in_ms: null } },
    {
      JoinPairingInvite: {
        invite_token: "chariox-invite-v1.client",
        subject_id: "client-2",
        public_key_thumbprint: null,
        alias: "desk",
      },
    },
  ])
})

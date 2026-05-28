import Foundation
import Testing
@testable import ArrobaFeature

@MainActor
@Test func defaultWorkspacePathsAreNotUserSpecific() {
    let defaults = UserDefaults(suiteName: "ArrobaAppModelTests.defaults")!
    defaults.removePersistentDomain(forName: "ArrobaAppModelTests.defaults")
    let model = ArrobaAppModel(client: SequencedMockKernelClient(responses: []), defaults: defaults)

    #expect(model.workspacePath.isEmpty)
    #expect(model.worktreePath.isEmpty)
}

@MainActor
@Test func refreshSessionsSelectsMostRecentSession() async {
    let client = MockKernelClient(response: .sessionsListed([
        RuntimeSession.fixture(id: "older-session", lastUsedAtMs: 100),
        RuntimeSession.fixture(id: "newer-session", lastUsedAtMs: 200),
    ]))
    let defaults = UserDefaults(suiteName: "ArrobaAppModelTests.refresh")!
    defaults.removePersistentDomain(forName: "ArrobaAppModelTests.refresh")
    let model = ArrobaAppModel(client: client, defaults: defaults)

    await model.refreshSessions()

    #expect(model.connectionState == .connected)
    #expect(model.sessions.map(\.id) == ["newer-session", "older-session"])
    #expect(model.selectedSessionID == "newer-session")
}

@MainActor
@Test func attachSelectedSessionStoresAttachmentAndStartsStream() async {
    let session = RuntimeSession.fixture(id: "session-1", lastUsedAtMs: 200)
    let client = MockKernelClient(
        response: .sessionAttached(RuntimeAttachment(id: "attachment-1", sessionID: session.id)),
        eventFrames: [
            KernelEventFrame(
                type: "event",
                eventID: 9,
                event: .heartbeat(sessionID: session.id)
            ),
            KernelEventFrame(
                type: "event",
                eventID: 10,
                event: .terminalOutput([
                    TerminalOutputRecord(
                        agentID: "agent-1",
                        kind: "provider_output",
                        mergeKey: nil,
                        bytes: Array("hello".utf8)
                    ),
                ])
            ),
        ]
    )
    let defaults = UserDefaults(suiteName: "ArrobaAppModelTests.attach")!
    defaults.removePersistentDomain(forName: "ArrobaAppModelTests.attach")
    let model = ArrobaAppModel(client: client, defaults: defaults)
    model.selectSession(session)

    await model.attachSelectedSession()
    try? await Task.sleep(for: .milliseconds(10))

    #expect(model.connectionState == .connected)
    #expect(model.activeAttachment?.id == "attachment-1")
    #expect(model.subscribedSessionID == "session-1")
    #expect(model.lastEventID == 10)
    #expect(model.lastHeartbeatAt != nil)
    #expect(model.transcriptEntries.last?.text == "hello")
}

@MainActor
@Test func terminalOutputChunksCoalesceByAgentAndKind() async {
    let session = RuntimeSession.fixture(id: "session-1", lastUsedAtMs: 200)
    let client = MockKernelClient(
        response: .sessionAttached(RuntimeAttachment(id: "attachment-1", sessionID: session.id)),
        eventFrames: [
            KernelEventFrame(
                type: "event",
                eventID: 1,
                event: .terminalOutput([
                    TerminalOutputRecord(
                        agentID: "agent-1",
                        kind: "provider_output",
                        mergeKey: nil,
                        bytes: Array("IOS".utf8)
                    ),
                ])
            ),
            KernelEventFrame(
                type: "event",
                eventID: 2,
                event: .terminalOutput([
                    TerminalOutputRecord(
                        agentID: "agent-1",
                        kind: "provider_output",
                        mergeKey: nil,
                        bytes: Array("_KERNEL_CONNECTED".utf8)
                    ),
                ])
            ),
        ]
    )
    let defaults = UserDefaults(suiteName: "ArrobaAppModelTests.coalesce")!
    defaults.removePersistentDomain(forName: "ArrobaAppModelTests.coalesce")
    let model = ArrobaAppModel(client: client, defaults: defaults)
    model.selectSession(session)

    await model.attachSelectedSession()
    try? await Task.sleep(for: .milliseconds(10))

    #expect(model.transcriptEntries.count == 1)
    #expect(model.transcriptEntries.last?.text == "IOS_KERNEL_CONNECTED")
}

@MainActor
@Test func terminalOutputRecordsCoalesceBeforeTranscriptMutation() async {
    let session = RuntimeSession.fixture(id: "session-1", lastUsedAtMs: 200)
    let client = MockKernelClient(
        response: .sessionAttached(RuntimeAttachment(id: "attachment-1", sessionID: session.id)),
        eventFrames: [
            KernelEventFrame(
                type: "event",
                eventID: 1,
                event: .terminalOutput([
                    TerminalOutputRecord(
                        agentID: "agent-1",
                        kind: "provider_output",
                        mergeKey: nil,
                        bytes: Array("A".utf8)
                    ),
                    TerminalOutputRecord(
                        agentID: "agent-1",
                        kind: "provider_output",
                        mergeKey: nil,
                        bytes: Array("B".utf8)
                    ),
                    TerminalOutputRecord(
                        agentID: "agent-1",
                        kind: "provider_reasoning",
                        mergeKey: nil,
                        bytes: Array("thinking".utf8)
                    ),
                    TerminalOutputRecord(
                        agentID: "agent-1",
                        kind: "provider_output",
                        mergeKey: nil,
                        bytes: Array("C".utf8)
                    ),
                    TerminalOutputRecord(
                        agentID: "agent-2",
                        kind: "provider_output",
                        mergeKey: nil,
                        bytes: Array("D".utf8)
                    ),
                ])
            ),
        ]
    )
    let defaults = UserDefaults(suiteName: "ArrobaAppModelTests.terminalBudget")!
    defaults.removePersistentDomain(forName: "ArrobaAppModelTests.terminalBudget")
    let model = ArrobaAppModel(client: client, defaults: defaults)
    model.selectSession(session)

    await model.attachSelectedSession()
    try? await Task.sleep(for: .milliseconds(10))

    #expect(model.transcriptEntries.map(\.text) == ["AB", "thinking", "C", "D"])
}

@MainActor
@Test func failedDetachKeepsLocalAttachmentState() async {
    let session = RuntimeSession.fixture(id: "session-1", lastUsedAtMs: 200)
    let client = SequencedMockKernelClient(responses: [
        .sessionAttached(RuntimeAttachment(id: "attachment-1", sessionID: session.id)),
        .sessionHistory([]),
        .unknown("DetachFailed"),
    ])
    let defaults = UserDefaults(suiteName: "ArrobaAppModelTests.detachFailure")!
    defaults.removePersistentDomain(forName: "ArrobaAppModelTests.detachFailure")
    let model = ArrobaAppModel(client: client, defaults: defaults)
    model.selectSession(session)

    await model.attachSelectedSession()
    await model.detachActiveSession()

    #expect(model.connectionState == .failed)
    #expect(model.activeAttachment?.id == "attachment-1")
    #expect(model.subscribedSessionID == "session-1")
}

@MainActor
@Test func heartbeatStaleDetectionMarksQuietLiveStream() async throws {
    let session = RuntimeSession.fixture(id: "session-1", lastUsedAtMs: 200)
    let client = MockKernelClient(
        response: .sessionAttached(RuntimeAttachment(id: "attachment-1", sessionID: session.id)),
        eventFrames: [
            KernelEventFrame(
                type: "event",
                eventID: 9,
                event: .heartbeat(sessionID: session.id)
            ),
        ],
        finishEvents: false
    )
    let defaults = UserDefaults(suiteName: "ArrobaAppModelTests.heartbeatStale")!
    defaults.removePersistentDomain(forName: "ArrobaAppModelTests.heartbeatStale")
    let model = ArrobaAppModel(
        client: client,
        defaults: defaults,
        heartbeatStaleAfterSeconds: 0.001
    )
    model.selectSession(session)

    await model.attachSelectedSession()
    try? await Task.sleep(for: .milliseconds(10))
    let lastHeartbeatAt = try #require(model.lastHeartbeatAt)
    model.evaluateHeartbeatStaleness(now: lastHeartbeatAt.addingTimeInterval(1))

    #expect(model.eventStreamState == .stale)
    #expect(model.statusMessage.contains("No kernel heartbeat"))
}

@MainActor
@Test func submitPromptClearsDraftAfterKernelAcceptance() async {
    let session = RuntimeSession.fixture(id: "session-1", lastUsedAtMs: 200)
    let client = SequencedMockKernelClient(responses: [
        .sessionAttached(RuntimeAttachment(id: "attachment-1", sessionID: session.id)),
        .sessionHistory([]),
        .promptSubmitted(session),
    ])
    let defaults = UserDefaults(suiteName: "ArrobaAppModelTests.submit")!
    defaults.removePersistentDomain(forName: "ArrobaAppModelTests.submit")
    let model = ArrobaAppModel(client: client, defaults: defaults)
    model.selectSession(session)

    await model.attachSelectedSession()
    model.promptDraft = "Build the iOS app"
    await model.submitPrompt()

    #expect(model.connectionState == .connected)
    #expect(model.promptDraft.isEmpty)
    #expect(model.statusMessage == "Prompt submitted.")
}

@MainActor
@Test func slashSessionListRefreshesSessionsAndWritesNotice() async {
    let client = MockKernelClient(response: .sessionsListed([
        RuntimeSession.fixture(id: "session-1", lastUsedAtMs: 200),
    ]))
    let defaults = UserDefaults(suiteName: "ArrobaAppModelTests.command.sessionList")!
    defaults.removePersistentDomain(forName: "ArrobaAppModelTests.command.sessionList")
    let model = ArrobaAppModel(client: client, defaults: defaults)
    model.promptDraft = "/session list"

    await model.submitPrompt()

    #expect(model.connectionState == .connected)
    #expect(model.promptDraft.isEmpty)
    #expect(model.sessions.map(\.id) == ["session-1"])
    #expect(model.transcriptEntries.last?.text.contains("Sessions") == true)
}

@MainActor
@Test func slashSessionDeleteRemovesSelectedSession() async {
    let session = RuntimeSession.fixture(id: "session-1", lastUsedAtMs: 200)
    let client = SequencedMockKernelClient(responses: [
        .sessionDeleted(session),
    ])
    let defaults = UserDefaults(suiteName: "ArrobaAppModelTests.command.sessionDelete")!
    defaults.removePersistentDomain(forName: "ArrobaAppModelTests.command.sessionDelete")
    let model = ArrobaAppModel(client: client, defaults: defaults)
    model.selectSession(session)
    model.promptDraft = "/session delete"

    await model.submitPrompt()

    #expect(model.connectionState == .connected)
    #expect(model.promptDraft.isEmpty)
    #expect(model.sessions.isEmpty)
    #expect(model.statusMessage == "Deleted session \(session.shortDisplayID).")
}

@MainActor
@Test func focusAgentUpdatesSelectedSessionImmediately() async {
    let builder = AgentInstance.fixture(id: "agent-1", agentRef: "1", alias: "builder", state: "Idle")
    let reviewer = AgentInstance.fixture(id: "agent-2", agentRef: "2", alias: "reviewer", state: "Idle")
    let focusedReviewer = AgentInstance.fixture(id: "agent-2", agentRef: "2", alias: "reviewer", state: "Focused")
    let session = RuntimeSession.fixture(
        id: "session-1",
        lastUsedAtMs: 200,
        focusedAgentID: builder.id,
        agents: [builder, reviewer]
    )
    let client = SequencedMockKernelClient(responses: [
        .agentFocused(focusedReviewer),
    ])
    let defaults = UserDefaults(suiteName: "ArrobaAppModelTests.focus")!
    defaults.removePersistentDomain(forName: "ArrobaAppModelTests.focus")
    let model = ArrobaAppModel(client: client, defaults: defaults)
    model.selectSession(session)

    await model.focusAgent(reviewer)

    #expect(model.connectionState == .connected)
    #expect(model.selectedSession?.focusedAgentID == focusedReviewer.id)
    #expect(model.selectedSession?.focusedAgent?.state == "Focused")
    #expect(model.statusMessage == "Focused reviewer.")
}

@MainActor
@Test func slashAgentFocusResolvesAliasAndClearsDraft() async {
    let builder = AgentInstance.fixture(id: "agent-1", agentRef: "1", alias: "builder", state: "Idle")
    let reviewer = AgentInstance.fixture(id: "agent-2", agentRef: "2", alias: "reviewer", state: "Idle")
    let focusedReviewer = AgentInstance.fixture(id: "agent-2", agentRef: "2", alias: "reviewer", state: "Focused")
    let session = RuntimeSession.fixture(
        id: "session-1",
        lastUsedAtMs: 200,
        focusedAgentID: builder.id,
        agents: [builder, reviewer]
    )
    let client = SequencedMockKernelClient(responses: [
        .agentFocused(focusedReviewer),
    ])
    let defaults = UserDefaults(suiteName: "ArrobaAppModelTests.command.agentFocus")!
    defaults.removePersistentDomain(forName: "ArrobaAppModelTests.command.agentFocus")
    let model = ArrobaAppModel(client: client, defaults: defaults)
    model.selectSession(session)
    model.promptDraft = "/agent focus reviewer"

    await model.submitPrompt()

    #expect(model.connectionState == .connected)
    #expect(model.promptDraft.isEmpty)
    #expect(model.selectedSession?.focusedAgentID == focusedReviewer.id)
}

@MainActor
@Test func spawnAgentAddsReturnedAgentAndFocusesIt() async {
    let builder = AgentInstance.fixture(id: "agent-1", agentRef: "1", alias: "builder", state: "Focused")
    let reviewer = AgentInstance.fixture(id: "agent-2", agentRef: "2", alias: "reviewer", state: "Focused")
    let session = RuntimeSession.fixture(
        id: "session-1",
        lastUsedAtMs: 200,
        focusedAgentID: builder.id,
        agents: [builder]
    )
    let client = SequencedMockKernelClient(responses: [
        .agentSpawned(reviewer),
    ])
    let defaults = UserDefaults(suiteName: "ArrobaAppModelTests.spawn")!
    defaults.removePersistentDomain(forName: "ArrobaAppModelTests.spawn")
    let model = ArrobaAppModel(client: client, defaults: defaults)
    model.selectSession(session)

    await model.spawnAgent(alias: "reviewer")

    #expect(model.connectionState == .connected)
    #expect(model.selectedSession?.agents.map(\.id) == ["agent-1", "agent-2"])
    #expect(model.selectedSession?.focusedAgentID == "agent-2")
    #expect(model.statusMessage == "Spawned agent reviewer.")
}

@MainActor
@Test func slashAgentDestroyRemovesTargetAndRefocusesRemainingAgent() async {
    let builder = AgentInstance.fixture(id: "agent-1", agentRef: "1", alias: "builder", state: "Idle")
    let reviewer = AgentInstance.fixture(id: "agent-2", agentRef: "2", alias: "reviewer", state: "Focused")
    let session = RuntimeSession.fixture(
        id: "session-1",
        lastUsedAtMs: 200,
        focusedAgentID: reviewer.id,
        agents: [builder, reviewer]
    )
    let client = SequencedMockKernelClient(responses: [
        .agentDestroyed(reviewer),
    ])
    let defaults = UserDefaults(suiteName: "ArrobaAppModelTests.destroy")!
    defaults.removePersistentDomain(forName: "ArrobaAppModelTests.destroy")
    let model = ArrobaAppModel(client: client, defaults: defaults)
    model.selectSession(session)
    model.promptDraft = "/agent destroy reviewer"

    await model.submitPrompt()

    #expect(model.connectionState == .connected)
    #expect(model.promptDraft.isEmpty)
    #expect(model.selectedSession?.agents.map(\.id) == ["agent-1"])
    #expect(model.selectedSession?.focusedAgentID == "agent-1")
}

@MainActor
@Test func slashAgentModeAppliesKernelUpdatedSession() async {
    let builder = AgentInstance.fixture(id: "agent-1", agentRef: "1", alias: "builder", state: "Focused")
    let updatedBuilder = AgentInstance.fixture(
        id: "agent-1",
        agentRef: "1",
        alias: "builder",
        state: "Focused",
        executionModeOverride: "plan"
    )
    let session = RuntimeSession.fixture(
        id: "session-1",
        lastUsedAtMs: 200,
        focusedAgentID: builder.id,
        agents: [builder]
    )
    let updatedSession = session.replacingAgents([updatedBuilder], focusedAgentID: updatedBuilder.id)
    let client = SequencedMockKernelClient(responses: [
        .agentConfigUpdated(agent: updatedBuilder, session: updatedSession),
    ])
    let defaults = UserDefaults(suiteName: "ArrobaAppModelTests.mode")!
    defaults.removePersistentDomain(forName: "ArrobaAppModelTests.mode")
    let model = ArrobaAppModel(client: client, defaults: defaults)
    model.selectSession(session)
    model.promptDraft = "/agent mode plan"

    await model.submitPrompt()

    #expect(model.connectionState == .connected)
    #expect(model.promptDraft.isEmpty)
    #expect(model.selectedSession?.focusedAgent?.executionModeOverride == "plan")
    #expect(model.statusMessage == "builder mode = plan.")
}

@MainActor
@Test func slashSessionModeUpdatesKernelBackedConfig() async {
    let session = RuntimeSession.fixture(id: "session-1", lastUsedAtMs: 200)
    let updatedSession = RuntimeSession.fixture(
        id: "session-1",
        lastUsedAtMs: 201,
        configState: SessionConfigState(
            version: 1,
            values: ["agents.mode": "plan"],
            updatedByAttachmentID: "attachment-1"
        )
    )
    let client = SequencedMockKernelClient(responses: [
        .sessionAttached(RuntimeAttachment(id: "attachment-1", sessionID: session.id)),
        .sessionHistory([]),
        .sessionConfigUpdated(
            session: updatedSession,
            config: updatedSession.configState!
        ),
    ])
    let defaults = UserDefaults(suiteName: "ArrobaAppModelTests.sessionMode")!
    defaults.removePersistentDomain(forName: "ArrobaAppModelTests.sessionMode")
    let model = ArrobaAppModel(client: client, defaults: defaults)
    model.selectSession(session)

    await model.attachSelectedSession()
    model.promptDraft = "/session mode plan"
    await model.submitPrompt()

    #expect(model.connectionState == .connected)
    #expect(model.promptDraft.isEmpty)
    #expect(model.selectedSession?.configState?.values["agents.mode"] == "plan")
    #expect(model.statusMessage == "Session mode = plan.")
}

@MainActor
@Test func slashProviderListLoadsCatalogAndWritesNotice() async {
    let catalog = ProviderCatalog.fixture()
    let client = SequencedMockKernelClient(responses: [
        .providerCatalog(catalog),
    ])
    let defaults = UserDefaults(suiteName: "ArrobaAppModelTests.providerList")!
    defaults.removePersistentDomain(forName: "ArrobaAppModelTests.providerList")
    let model = ArrobaAppModel(client: client, defaults: defaults)
    model.promptDraft = "/provider list"

    await model.submitPrompt()

    #expect(model.connectionState == .connected)
    #expect(model.promptDraft.isEmpty)
    #expect(model.providerCatalog?.connected == ["codex"])
    #expect(model.transcriptEntries.last?.text.contains("Providers") == true)
}

@MainActor
@Test func slashProviderAuthLoadsStatusAndWritesNotice() async {
    let status = ProviderAuthStatus(
        provider: "codex",
        authState: "authenticated",
        accountProfile: "miguel",
        loginHint: nil,
        detectedVersion: "1.2.3"
    )
    let client = SequencedMockKernelClient(responses: [
        .providerAuthStatus(status),
    ])
    let defaults = UserDefaults(suiteName: "ArrobaAppModelTests.providerAuth")!
    defaults.removePersistentDomain(forName: "ArrobaAppModelTests.providerAuth")
    let model = ArrobaAppModel(client: client, defaults: defaults)
    model.promptDraft = "/provider auth codex"

    await model.submitPrompt()

    #expect(model.connectionState == .connected)
    #expect(model.promptDraft.isEmpty)
    #expect(model.providerAuthStatuses["codex"]?.accountProfile == "miguel")
    #expect(model.transcriptEntries.last?.text == "codex: authenticated as miguel • version 1.2.3")
}

@MainActor
@Test func slashProviderLoginAndLogoutUseKernelRequests() async {
    let login = ProviderLoginStart(
        provider: "codex",
        loginKind: "device",
        loginID: "login-1",
        authURL: nil,
        verificationURL: "https://example.com/activate",
        userCode: "ABCD-EFGH"
    )
    let client = SequencedMockKernelClient(responses: [
        .providerLoginStarted(login),
        .providerLoggedOut("codex"),
    ])
    let defaults = UserDefaults(suiteName: "ArrobaAppModelTests.providerLoginLogout")!
    defaults.removePersistentDomain(forName: "ArrobaAppModelTests.providerLoginLogout")
    let model = ArrobaAppModel(client: client, defaults: defaults)
    model.promptDraft = "/provider login codex"

    await model.submitPrompt()
    #expect(model.connectionState == .connected)
    #expect(model.promptDraft.isEmpty)
    #expect(model.transcriptEntries.last?.text.contains("ABCD-EFGH") == true)

    model.promptDraft = "/provider logout codex"
    await model.submitPrompt()
    #expect(model.connectionState == .connected)
    #expect(model.promptDraft.isEmpty)
    #expect(model.transcriptEntries.last?.text == "codex: logged out")
}

@MainActor
@Test func slashModelVariantAndViewPersistClientSelection() async {
    let defaults = UserDefaults(suiteName: "ArrobaAppModelTests.clientSelection")!
    defaults.removePersistentDomain(forName: "ArrobaAppModelTests.clientSelection")
    let model = ArrobaAppModel(client: SequencedMockKernelClient(responses: []), defaults: defaults)

    model.promptDraft = "/provider codex"
    await model.submitPrompt()
    model.promptDraft = "/model gpt-5.4"
    await model.submitPrompt()
    model.promptDraft = "/variant high"
    await model.submitPrompt()
    model.promptDraft = "/view split"
    await model.submitPrompt()

    #expect(model.selectedProviderID == "codex")
    #expect(model.selectedModelID == "gpt-5.4")
    #expect(model.selectedVariantID == "high")
    #expect(model.responseLayout == .split)
    #expect(defaults.string(forKey: "dev.arroba.ios.providerID") == "codex")
    #expect(defaults.string(forKey: "dev.arroba.ios.modelID") == "gpt-5.4")
    #expect(defaults.string(forKey: "dev.arroba.ios.variantID") == "high")
    #expect(defaults.string(forKey: "dev.arroba.ios.responseLayout") == "split")
}

@MainActor
@Test func slashMcpListLoadsServersAndWritesNotice() async {
    let client = SequencedMockKernelClient(responses: [
        .mcpServersListed([
            ArrobaMcpServerConfig(
                name: "playwright",
                transport: ["stdio": .object([:])],
                enabled: true,
                required: nil,
                startupTimeoutSeconds: nil,
                toolTimeoutSeconds: nil,
                enabledTools: nil,
                disabledTools: nil,
                tools: nil
            ),
        ]),
    ])
    let defaults = UserDefaults(suiteName: "ArrobaAppModelTests.mcpList")!
    defaults.removePersistentDomain(forName: "ArrobaAppModelTests.mcpList")
    let model = ArrobaAppModel(client: client, defaults: defaults)
    model.promptDraft = "/mcp list"

    await model.submitPrompt()

    #expect(model.connectionState == .connected)
    #expect(model.promptDraft.isEmpty)
    #expect(model.mcpServers.map(\.name) == ["playwright"])
    #expect(model.transcriptEntries.last?.text.contains("MCP servers") == true)
}

@MainActor
@Test func slashSkillListLoadsSkillsAndWritesNotice() async {
    let client = SequencedMockKernelClient(responses: [
        .skillsListed([
            ArrobaSkillMetadata(
                name: "swiftui-expert",
                description: "SwiftUI guidance",
                shortDescription: "SwiftUI",
                path: "/skills/swiftui"
            ),
        ]),
    ])
    let defaults = UserDefaults(suiteName: "ArrobaAppModelTests.skillList")!
    defaults.removePersistentDomain(forName: "ArrobaAppModelTests.skillList")
    let model = ArrobaAppModel(client: client, defaults: defaults)
    model.promptDraft = "/skill list"

    await model.submitPrompt()

    #expect(model.connectionState == .connected)
    #expect(model.promptDraft.isEmpty)
    #expect(model.skills.map(\.name) == ["swiftui-expert"])
    #expect(model.transcriptEntries.last?.text.contains("Skills") == true)
}

@MainActor
@Test func slashWorkspaceSetUpdatesLocalSessionTargets() async {
    let defaults = UserDefaults(suiteName: "ArrobaAppModelTests.workspaceSet")!
    defaults.removePersistentDomain(forName: "ArrobaAppModelTests.workspaceSet")
    let model = ArrobaAppModel(
        client: SequencedMockKernelClient(responses: []),
        defaults: defaults
    )
    model.promptDraft = "/workspace set /tmp/repo"

    await model.submitPrompt()

    #expect(model.promptDraft.isEmpty)
    #expect(model.workspacePath == "/tmp/repo")
    #expect(model.worktreePath == "/tmp/repo")
    #expect(defaults.string(forKey: "dev.arroba.ios.workspacePath") == "/tmp/repo")
    #expect(model.transcriptEntries.last?.text == "Workspace/worktree path = /tmp/repo")
}

@MainActor
@Test func slashWorkspaceSyncCommandsUseKernelProtocol() async {
    let session = RuntimeSession.fixture(id: "session-sync", lastUsedAtMs: 300)
    let status = WorkspaceLiveSyncStatus(
        sessionID: session.id,
        mode: "tracked",
        footerState: "conflict",
        targets: [
            WorkspaceLiveSyncTargetStatus(
                linkID: "link-1",
                linkName: "shared",
                userID: "user-2",
                machineID: "machine-2",
                kernelID: "kernel-2",
                repoRoot: "/repo/peer",
                branch: "tracked-peer",
                repoFingerprint: "fingerprint-2",
                status: "ready",
                attachedAtMs: 42
            ),
        ],
        conflicts: [
            WorkspaceLiveSyncConflictSummary(
                conflictID: "conflict-1",
                linkID: "link-1",
                sourceAgentID: "agent-1",
                targetUserID: "user-2",
                targetRepoRoot: "/repo",
                path: "src/app.swift",
                nextAction: "reconcile and retry"
            ),
        ],
        ignore: WorkspaceLiveSyncIgnoreStatus(
            ignoreFile: "/repo/.arrobaignore",
            rules: ["ignored/**", "*.secret"],
            forceExcludes: [".git/**", ".arroba/**"]
        )
    )
    let client = SequencedMockKernelClient(responses: [
        .workspaceLiveSyncStatus(status),
        .workspaceLiveSyncStatus(status),
        .workspaceLiveSyncStatus(status),
        .workspaceLiveSyncStatus(status),
        .userConfigUpdated(path: "/tmp/config.json", effects: []),
        .userConfigUpdated(path: "/tmp/config.json", effects: []),
        .userConfigUpdated(path: "/tmp/config.json", effects: []),
        .workspaceLinkAttached(session: session),
    ])
    let defaults = UserDefaults(suiteName: "ArrobaAppModelTests.workspaceSync")!
    defaults.removePersistentDomain(forName: "ArrobaAppModelTests.workspaceSync")
    let model = ArrobaAppModel(client: client, defaults: defaults)
    model.selectSession(session)
    model.promptDraft = "/workspace sync status"

    await model.submitPrompt()

    #expect(model.promptDraft.isEmpty)
    #expect(model.transcriptEntries.last?.text.contains("rules: 2") == true)

    model.promptDraft = "/workspace sync conflicts"

    await model.submitPrompt()

    #expect(model.promptDraft.isEmpty)
    #expect(model.transcriptEntries.last?.text.contains("src/app.swift") == true)

    model.promptDraft = "/workspace sync targets"
    await model.submitPrompt()

    #expect(model.promptDraft.isEmpty)
    #expect(model.transcriptEntries.last?.text.contains("user-2 /repo/peer @ tracked-peer") == true)

    model.promptDraft = "/workspace sync ignore"
    await model.submitPrompt()

    #expect(model.promptDraft.isEmpty)
    #expect(model.transcriptEntries.last?.text.contains("rules: ignored/**, *.secret") == true)
    #expect(model.transcriptEntries.last?.text.contains("force excludes: .git/**, .arroba/**") == true)

    model.promptDraft = "/workspace sync mode managed"
    await model.submitPrompt()

    #expect(model.promptDraft.isEmpty)
    #expect(model.transcriptEntries.last?.text == "Workspace live sync mode = managed")

    model.promptDraft = "/workspace sync enable tracked"
    await model.submitPrompt()

    #expect(model.promptDraft.isEmpty)
    #expect(model.transcriptEntries.last?.text == "Workspace live sync mode = tracked")

    model.promptDraft = "/workspace sync disable"
    await model.submitPrompt()

    #expect(model.promptDraft.isEmpty)
    #expect(model.transcriptEntries.last?.text == "Workspace live sync mode = unrestricted")

    model.promptDraft = "/workspace sync link shared"
    await model.submitPrompt()

    #expect(model.promptDraft.isEmpty)
    #expect(model.transcriptEntries.last?.text == "Workspace live sync linked shared -> /repo. Recommended mode: managed.")
}

@MainActor
@Test func slashConfigWorkspaceLiveSyncUsesKernelProtocol() async {
    let client = SequencedMockKernelClient(responses: [
        .userConfigUpdated(path: "/tmp/config.json", effects: []),
        .userConfigUpdated(path: "/tmp/config.json", effects: []),
        .userConfigUpdated(path: "/tmp/config.json", effects: []),
        .userConfigUpdated(path: "/tmp/config.json", effects: []),
    ])
    let defaults = UserDefaults(suiteName: "ArrobaAppModelTests.configWorkspaceLiveSync")!
    defaults.removePersistentDomain(forName: "ArrobaAppModelTests.configWorkspaceLiveSync")
    let model = ArrobaAppModel(client: client, defaults: defaults)

    model.promptDraft = "/config workspace-live-sync"
    await model.submitPrompt()

    #expect(model.promptDraft.isEmpty)
    #expect(model.transcriptEntries.last?.text == "Workspace live sync set to managed")

    model.promptDraft = "/config workspace-live-sync unrestricted"
    await model.submitPrompt()

    #expect(model.promptDraft.isEmpty)
    #expect(model.transcriptEntries.last?.text == "Workspace live sync set to unrestricted")

    model.promptDraft = "/config workspace-live-sync required"
    await model.submitPrompt()

    #expect(model.promptDraft.isEmpty)
    #expect(model.transcriptEntries.last?.text == "Workspace live sync set to required")

    model.promptDraft = "/config workspace-live-sync tracked"
    await model.submitPrompt()

    #expect(model.promptDraft.isEmpty)
    #expect(model.transcriptEntries.last?.text == "Workspace live sync set to tracked")
}

@MainActor
@Test func respondToInteractionUpdatesSessionProjection() async {
    let interaction = RuntimeInteraction.fixture(id: "interaction-1", agentID: "agent-1")
    let session = RuntimeSession.fixture(
        id: "session-1",
        lastUsedAtMs: 200,
        activeInteractions: [interaction]
    )
    let updatedSession = RuntimeSession.fixture(id: "session-1", lastUsedAtMs: 201)
    let client = SequencedMockKernelClient(responses: [
        .interactionResponded(interactionID: interaction.id, session: updatedSession),
    ])
    let defaults = UserDefaults(suiteName: "ArrobaAppModelTests.interaction")!
    defaults.removePersistentDomain(forName: "ArrobaAppModelTests.interaction")
    let model = ArrobaAppModel(client: client, defaults: defaults)
    model.selectSession(session)

    await model.respondToInteraction(interaction, choice: interaction.choices[0])

    #expect(model.connectionState == .connected)
    #expect(model.selectedSession?.activeInteractions.isEmpty == true)
    #expect(model.statusMessage == "Responded to Permission request.")
}

private extension ProviderCatalog {
    static func fixture() -> ProviderCatalog {
        ProviderCatalog(
            all: [
                ProviderInfo(
                    id: "codex",
                    name: "Codex",
                    remoteMachineAliases: [],
                    models: [
                        "gpt-5.4": ProviderModel(
                            id: "gpt-5.4",
                            name: "GPT-5.4",
                            status: "active",
                            limit: nil,
                            variants: ["medium": .object([:])]
                        ),
                    ]
                ),
            ],
            default: ["codex": "gpt-5.4"],
            connected: ["codex"]
        )
    }
}

@Test func commandCenterCatalogOffersSessionAndAgentSuggestions() {
    let builder = AgentInstance.fixture(id: "agent-1", agentRef: "1", alias: "builder", state: "Idle")
    let session = RuntimeSession.fixture(
        id: "session-1",
        lastUsedAtMs: 200,
        focusedAgentID: builder.id,
        agents: [builder]
    )

    #expect(CommandCenterCatalog.items(matching: "/", session: nil).map(\.id).contains("session"))
    #expect(CommandCenterCatalog.items(matching: "/session", session: nil).map(\.id).contains("session-list"))
    #expect(CommandCenterCatalog.items(matching: "/agent", session: session).map(\.id).contains("agent-focus"))
    #expect(CommandCenterCatalog.items(matching: "/agent focus bui", session: session).first?.value == "/agent focus agent-1")
    #expect(CommandCenterCatalog.items(matching: "/workspace", session: session).map(\.id).contains("workspace-set"))
    #expect(CommandCenterCatalog.items(matching: "/workspace sync", session: session).map(\.id).contains("workspace-sync-status"))
    #expect(CommandCenterCatalog.items(matching: "/workspace sync", session: session).map(\.id).contains("workspace-sync-targets"))
    #expect(CommandCenterCatalog.items(matching: "/workspace sync", session: session).map(\.id).contains("workspace-sync-enable-managed"))
    #expect(CommandCenterCatalog.items(matching: "/workspace sync", session: session).map(\.id).contains("workspace-sync-enable-tracked"))
    #expect(CommandCenterCatalog.items(matching: "/workspace sync", session: session).map(\.id).contains("workspace-sync-disable"))
    #expect(CommandCenterCatalog.items(matching: "/workspace sync", session: session).map(\.id).contains("workspace-sync-mode"))
    #expect(CommandCenterCatalog.items(matching: "/workspace sync", session: session).map(\.id).contains("workspace-sync-link"))
    #expect(CommandCenterCatalog.items(matching: "/workspace sync", session: session).map(\.id).contains("workspace-sync-conflicts"))
    #expect(CommandCenterCatalog.items(matching: "/workspace sync", session: session).map(\.id).contains("workspace-sync-ignore"))
    #expect(CommandCenterCatalog.items(matching: "/", session: session).map(\.id).contains("config"))
    let configIds = CommandCenterCatalog.items(matching: "/config workspace-live-sync", session: session).map(\.id)
    #expect(configIds.contains("config-workspace-live-sync-required"))
    #expect(configIds.contains("config-workspace-live-sync-tracked"))
    #expect(CommandCenterCatalog.items(matching: "/config workspace-live-sync", session: session).map(\.id).contains("config-workspace-live-sync-unrestricted"))
    #expect(CommandCenterCatalog.items(matching: "/", session: session).map(\.id).contains("model"))
    #expect(CommandCenterCatalog.items(matching: "/provider", session: session).map(\.id).contains("provider-login"))
    #expect(CommandCenterCatalog.items(matching: "/view", session: session).map(\.id).contains("view-split"))
}

private struct MockKernelClient: KernelClientProtocol {
    let response: LocalDaemonResponse
    var eventFrames: [KernelEventFrame] = []
    var finishEvents = true

    func send(_ request: LocalDaemonRequest, to endpoint: URL) async throws -> LocalDaemonResponse {
        response
    }

    func events(
        sessionID: String,
        attachmentID: String,
        endpoint: URL,
        resumeFromEventID: Int64?
    ) -> AsyncThrowingStream<KernelEventFrame, Error> {
        AsyncThrowingStream { continuation in
            for frame in eventFrames {
                continuation.yield(frame)
            }
            if finishEvents {
                continuation.finish()
            }
        }
    }
}

private struct SequencedMockKernelClient: KernelClientProtocol {
    private let queue: ResponseQueue

    init(responses: [LocalDaemonResponse]) {
        queue = ResponseQueue(responses: responses)
    }

    func send(_ request: LocalDaemonRequest, to endpoint: URL) async throws -> LocalDaemonResponse {
        await queue.next()
    }

    func events(
        sessionID: String,
        attachmentID: String,
        endpoint: URL,
        resumeFromEventID: Int64?
    ) -> AsyncThrowingStream<KernelEventFrame, Error> {
        AsyncThrowingStream { continuation in
            continuation.finish()
        }
    }
}

private actor ResponseQueue {
    private var responses: [LocalDaemonResponse]

    init(responses: [LocalDaemonResponse]) {
        self.responses = responses
    }

    func next() -> LocalDaemonResponse {
        responses.isEmpty ? .unknown("NoMockResponse") : responses.removeFirst()
    }
}

private extension RuntimeSession {
    static func fixture(
        id: String,
        lastUsedAtMs: Int64,
        focusedAgentID: String? = nil,
        agents: [AgentInstance] = [],
        configState: SessionConfigState? = nil,
        activePrompt: PromptQueueItem? = nil,
        queuedPrompts: [PromptQueueItem] = [],
        promptStates: [String: AgentPromptState] = [:],
        activeInteractions: [RuntimeInteraction] = []
    ) -> RuntimeSession {
        RuntimeSession(
            id: id,
            alias: nil,
            workspaceID: "/repo",
            worktreeID: "/repo",
            status: "Active",
            configState: configState,
            activePrompt: activePrompt,
            queuedPrompts: queuedPrompts,
            promptStates: promptStates,
            activeInteractions: activeInteractions,
            focusedAgentID: focusedAgentID,
            agents: agents,
            createdAtMs: 1,
            lastUsedAtMs: lastUsedAtMs
        )
    }
}

private extension RuntimeInteraction {
    static func fixture(id: String, agentID: String) -> RuntimeInteraction {
        RuntimeInteraction(
            id: id,
            agentID: agentID,
            kind: "permission",
            level: "warning",
            title: nil,
            message: "Allow command?",
            choices: [
                RuntimeInteractionChoice(
                    id: "approve",
                    label: "Approve",
                    reply: "approved",
                    style: "primary"
                ),
            ],
            timeoutSeconds: nil,
            defaultOnTimeout: nil,
            requestedAtMs: 1
        )
    }
}

private extension AgentInstance {
    static func fixture(
        id: String,
        agentRef: String,
        alias: String,
        state: String,
        executionModeOverride: String? = nil,
        permissionLevelOverride: String? = nil
    ) -> AgentInstance {
        AgentInstance(
            id: id,
            agentRef: agentRef,
            sessionID: "session-1",
            alias: alias,
            provider: "opencode",
            model: "default",
            effort: nil,
            executionModeOverride: executionModeOverride,
            permissionLevelOverride: permissionLevelOverride,
            worktreeID: nil,
            state: state,
            isProcessing: false
        )
    }
}

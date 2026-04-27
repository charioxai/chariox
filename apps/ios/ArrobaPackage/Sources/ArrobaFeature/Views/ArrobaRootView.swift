import SwiftUI

public struct ArrobaRootView: View {
    @Bindable private var model: ArrobaAppModel

    public init(model: ArrobaAppModel) {
        self.model = model
    }

    public var body: some View {
        GeometryReader { proxy in
            let isWide = proxy.size.width >= 760
            Group {
                if isWide {
                    HStack(spacing: 0) {
                        RuntimeDrawer(model: model)
                            .frame(width: min(340, proxy.size.width * 0.38))
                        Divider()
                            .overlay(ArrobaPalette.border)
                        WaitingRoomView(model: model)
                    }
                } else {
                    VStack(spacing: 0) {
                        WaitingRoomView(model: model)
                        Divider()
                            .overlay(ArrobaPalette.border)
                        RuntimeDrawer(model: model)
                            .frame(maxHeight: 250)
                    }
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .background(ArrobaPalette.background)
        }
        .preferredColorScheme(.dark)
        .task {
            await model.refreshSessions()
        }
    }
}

private struct WaitingRoomView: View {
    @Bindable var model: ArrobaAppModel
    @FocusState private var focusedField: Field?

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            header
            ScrollView {
                VStack(alignment: .leading, spacing: 24) {
                    connectionPanel
                    sessionMenu
                    PromptComposer(model: model)
                    InteractionPromptPanel(model: model)
                    TranscriptView(entries: model.transcriptEntries)
                }
                .padding(24)
            }
            GlobalFooter(state: model.connectionState, message: model.statusMessage)
        }
        .background(ArrobaPalette.panel)
    }

    private var header: some View {
        HStack(alignment: .lastTextBaseline) {
            Text("@")
                .font(.system(size: 48, weight: .heavy, design: .monospaced))
                .foregroundStyle(ArrobaPalette.orange)
            VStack(alignment: .leading, spacing: 2) {
                Text("ARROBA")
                    .font(.system(size: 30, weight: .bold, design: .monospaced))
                    .foregroundStyle(.white)
                Text("iOS kernel client")
                    .font(.system(.callout, design: .monospaced))
                    .foregroundStyle(ArrobaPalette.muted)
            }
            Spacer()
            StatusPill(state: model.connectionState)
        }
        .padding(.horizontal, 24)
        .padding(.vertical, 20)
        .background(ArrobaPalette.background)
        .accessibilityElement(children: .combine)
        .accessibilityLabel("Arroba iOS kernel client")
    }

    private var connectionPanel: some View {
        VStack(alignment: .leading, spacing: 14) {
            TerminalField(
                title: "Kernel",
                text: $model.kernelURLText,
                prompt: "ws://127.0.0.1:43118/kernel"
            )
            .focused($focusedField, equals: .kernel)

            HStack(spacing: 12) {
                Button {
                    Task { await model.refreshSessions() }
                } label: {
                    Label("Refresh", systemImage: "arrow.clockwise")
                }
                .buttonStyle(ArrobaCommandButtonStyle())
                .disabled(model.connectionState.isWorking)
                .accessibilityIdentifier("waiting-refresh")

                Button {
                    Task { await model.createSession() }
                } label: {
                    Label("New Session", systemImage: "plus")
                }
                .buttonStyle(ArrobaCommandButtonStyle(primary: true))
                .disabled(model.connectionState.isWorking)
                .accessibilityIdentifier("waiting-new-session")

                Button {
                    Task { await model.attachSelectedSession() }
                } label: {
                    Label("Attach", systemImage: "link")
                }
                .buttonStyle(ArrobaCommandButtonStyle())
                .disabled(model.connectionState.isWorking || model.selectedSession == nil)
                .accessibilityIdentifier("waiting-attach-session")

                Button {
                    Task { await model.detachActiveSession() }
                } label: {
                    Label("Detach", systemImage: "xmark")
                }
                .buttonStyle(ArrobaCommandButtonStyle())
                .disabled(model.connectionState.isWorking || model.activeAttachment == nil)
                .accessibilityIdentifier("waiting-detach-session")
            }
        }
    }

    private var sessionMenu: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text("Waiting Room")
                .font(.system(.headline, design: .monospaced))
                .foregroundStyle(.white)

            if model.sessions.isEmpty {
                EmptyStateRow()
            } else {
                VStack(spacing: 1) {
                    ForEach(model.sessions) { session in
                        SessionRow(
                            session: session,
                            selected: model.selectedSessionID == session.id
                        ) {
                            model.selectSession(session)
                        }
                    }
                }
                .background(ArrobaPalette.border)
                .clipShape(.rect(cornerRadius: 8))
            }
        }
    }

    private enum Field {
        case kernel
    }
}

private struct PromptComposer: View {
    @Bindable var model: ArrobaAppModel

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack {
                Text("Prompt")
                    .font(.system(.headline, design: .monospaced))
                    .foregroundStyle(.white)
                Spacer()
                Text(promptModeText)
                    .font(.system(.caption, design: .monospaced, weight: .semibold))
                    .foregroundStyle(promptModeColor)
            }

            if !model.commandCenterItems.isEmpty {
                CommandCenterPanel(items: model.commandCenterItems) { item in
                    Task { await model.executeCommandCenterItem(item) }
                }
            }

            TextEditor(text: $model.promptDraft)
                .arrobaTerminalEditor()
                .font(.system(.callout, design: .monospaced))
                .frame(minHeight: 92)
                .padding(10)
                .foregroundStyle(.white)
                .background(ArrobaPalette.field)
                .clipShape(.rect(cornerRadius: 8))
                .overlay(
                    RoundedRectangle(cornerRadius: 8)
                        .stroke(ArrobaPalette.border, lineWidth: 1)
                )
                .accessibilityLabel("Prompt")
                .accessibilityIdentifier("prompt-composer")

            HStack(spacing: 12) {
                Button {
                    Task { await model.submitPrompt() }
                } label: {
                    Label("Send", systemImage: "paperplane.fill")
                }
                .buttonStyle(ArrobaCommandButtonStyle(primary: true))
                .disabled(model.connectionState.isWorking || model.activeAttachment == nil)
                .accessibilityIdentifier("prompt-send")

                Button {
                    Task { await model.cancelActivePrompt() }
                } label: {
                    Label("Stop", systemImage: "stop.fill")
                }
                .buttonStyle(ArrobaCommandButtonStyle())
                .disabled(model.connectionState.isWorking || model.activeAttachment == nil)
                .accessibilityIdentifier("prompt-stop")
            }
        }
    }

    private var isCommandDraft: Bool {
        model.promptDraft.trimmingCharacters(in: .whitespacesAndNewlines).hasPrefix("/")
    }

    private var promptModeText: String {
        if isCommandDraft { return "command" }
        return model.activeAttachment == nil ? "attach required" : "focused agent"
    }

    private var promptModeColor: Color {
        model.activeAttachment == nil && !isCommandDraft ? ArrobaPalette.muted : ArrobaPalette.orange
    }
}

private struct CommandCenterPanel: View {
    let items: [CommandCenterItem]
    let select: (CommandCenterItem) -> Void

    var body: some View {
        VStack(spacing: 1) {
            ForEach(items.prefix(5)) { item in
                Button {
                    select(item)
                } label: {
                    HStack(alignment: .firstTextBaseline, spacing: 10) {
                        Text(item.label)
                            .font(.system(.caption, design: .monospaced, weight: .bold))
                            .foregroundStyle(ArrobaPalette.orange)
                            .frame(minWidth: 76, alignment: .leading)
                        Text(item.detail)
                            .font(.system(.caption, design: .monospaced))
                            .foregroundStyle(ArrobaPalette.muted)
                            .lineLimit(2)
                        Spacer()
                        Text(item.submitsImmediately ? "run" : "insert")
                            .font(.system(.caption2, design: .monospaced, weight: .bold))
                            .foregroundStyle(ArrobaPalette.muted)
                    }
                    .padding(.horizontal, 10)
                    .padding(.vertical, 9)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .background(ArrobaPalette.field)
                }
                .buttonStyle(.plain)
                .accessibilityIdentifier("command-center-\(item.id)")
            }
        }
        .background(ArrobaPalette.border)
        .clipShape(.rect(cornerRadius: 8))
        .accessibilityIdentifier("command-center")
    }
}

private struct TranscriptView: View {
    let entries: [TranscriptEntry]

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack {
                Text("Transcript")
                    .font(.system(.headline, design: .monospaced))
                    .foregroundStyle(.white)
                Spacer()
                Text("\(entries.count) event\(entries.count == 1 ? "" : "s")")
                    .font(.system(.caption, design: .monospaced, weight: .semibold))
                    .foregroundStyle(ArrobaPalette.muted)
            }

            ScrollViewReader { proxy in
                ScrollView {
                    LazyVStack(alignment: .leading, spacing: 10) {
                        if entries.isEmpty {
                            Text("Attach, send a prompt, and streamed kernel output will appear here.")
                                .font(.system(.callout, design: .monospaced))
                                .foregroundStyle(ArrobaPalette.muted)
                                .frame(maxWidth: .infinity, alignment: .leading)
                        } else {
                            ForEach(entries) { entry in
                                TranscriptRow(entry: entry)
                                    .id(entry.id)
                            }
                        }
                    }
                    .padding(12)
                }
                .frame(minHeight: 160, maxHeight: 280)
                .background(ArrobaPalette.field)
                .clipShape(.rect(cornerRadius: 8))
                .overlay(
                    RoundedRectangle(cornerRadius: 8)
                        .stroke(ArrobaPalette.border, lineWidth: 1)
                )
                .accessibilityIdentifier("transcript-view")
                .onChange(of: entries.last?.id) {
                    guard let id = entries.last?.id else { return }
                    withAnimation(.easeOut(duration: 0.2)) {
                        proxy.scrollTo(id, anchor: .bottom)
                    }
                }
            }
        }
    }
}

private struct InteractionPromptPanel: View {
    @Bindable var model: ArrobaAppModel

    var body: some View {
        let interactions = model.selectedSession?.activeInteractions ?? []
        if !interactions.isEmpty {
            VStack(alignment: .leading, spacing: 10) {
                HStack {
                    Text("Interaction")
                        .font(.system(.headline, design: .monospaced))
                        .foregroundStyle(.white)
                    Spacer()
                    Text("\(interactions.count) pending")
                        .font(.system(.caption, design: .monospaced, weight: .semibold))
                        .foregroundStyle(ArrobaPalette.muted)
                }

                VStack(spacing: 1) {
                    ForEach(interactions) { interaction in
                        InteractionPromptRow(
                            interaction: interaction,
                            agent: model.selectedSession?.agents.first { $0.id == interaction.agentID },
                            isWorking: model.connectionState.isWorking
                        ) { choice in
                            Task { await model.respondToInteraction(interaction, choice: choice) }
                        }
                    }
                }
                .background(ArrobaPalette.border)
                .clipShape(.rect(cornerRadius: 8))
            }
            .accessibilityIdentifier("interaction-prompt-panel")
        }
    }
}

private struct InteractionPromptRow: View {
    let interaction: RuntimeInteraction
    let agent: AgentInstance?
    let isWorking: Bool
    let choose: (RuntimeInteractionChoice) -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(alignment: .firstTextBaseline, spacing: 8) {
                Text(interaction.levelText)
                    .font(.system(.caption2, design: .monospaced, weight: .bold))
                    .foregroundStyle(levelColor)
                Text(interaction.displayTitle)
                    .font(.system(.callout, design: .monospaced, weight: .semibold))
                    .foregroundStyle(.white)
                Spacer()
                if let agent {
                    Text(agent.displayName)
                        .font(.system(.caption2, design: .monospaced, weight: .bold))
                        .foregroundStyle(ArrobaPalette.muted)
                }
            }
            Text(interaction.message)
                .font(.system(.caption, design: .monospaced))
                .foregroundStyle(ArrobaPalette.muted)
                .textSelection(.enabled)

            HStack(spacing: 8) {
                ForEach(interaction.choices) { choice in
                    Button {
                        choose(choice)
                    } label: {
                        Text(choice.label)
                            .lineLimit(1)
                    }
                    .buttonStyle(ArrobaCommandButtonStyle(primary: choice.style == "primary"))
                    .disabled(isWorking)
                    .accessibilityIdentifier("interaction-choice-\(choice.id)")
                }
            }
        }
        .padding(12)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(ArrobaPalette.field)
    }

    private var levelColor: Color {
        switch interaction.level {
        case "critical":
            ArrobaPalette.red
        case "warning":
            ArrobaPalette.orange
        default:
            ArrobaPalette.muted
        }
    }
}

private struct TranscriptRow: View {
    let entry: TranscriptEntry

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack(spacing: 8) {
                Text(entry.kind.rawValue.uppercased())
                    .font(.system(.caption2, design: .monospaced, weight: .bold))
                    .foregroundStyle(color)
                if let agentID = entry.agentID {
                    Text(agentID)
                        .font(.system(.caption2, design: .monospaced))
                        .foregroundStyle(ArrobaPalette.muted)
                }
                Spacer()
            }
            Text(entry.text)
                .font(.system(.caption, design: .monospaced))
                .foregroundStyle(.white)
                .textSelection(.enabled)
                .frame(maxWidth: .infinity, alignment: .leading)
        }
        .padding(.vertical, 2)
    }

    private var color: Color {
        switch entry.kind {
        case .error:
            ArrobaPalette.red
        case .notice, .completion, .status:
            ArrobaPalette.muted
        default:
            ArrobaPalette.orange
        }
    }
}

private struct RuntimeDrawer: View {
    @Bindable var model: ArrobaAppModel

    var body: some View {
        VStack(alignment: .leading, spacing: 18) {
            Text("Runtime")
                .font(.system(.title3, design: .monospaced, weight: .bold))
                .foregroundStyle(.white)

            VStack(alignment: .leading, spacing: 12) {
                TerminalField(
                    title: "Workspace",
                    text: $model.workspacePath,
                    prompt: "/path/to/repo on kernel host"
                )
                TerminalField(
                    title: "Worktree",
                    text: $model.worktreePath,
                    prompt: "/path/to/worktree on kernel host"
                )
            }

            Divider()
                .overlay(ArrobaPalette.border)

            SelectedSessionSummary(session: model.selectedSession)
            ProviderCatalogSummary(
                catalog: model.providerCatalog,
                authStatuses: model.providerAuthStatuses,
                mcpServers: model.mcpServers,
                skills: model.skills
            )
            AgentFocusPanel(
                session: model.selectedSession,
                isWorking: model.connectionState.isWorking,
                spawnAgent: {
                    Task { await model.spawnAgent() }
                },
                focusAgent: { agent in
                    Task { await model.focusAgent(agent) }
                },
                destroyAgent: { agent in
                    Task { await model.destroyAgent(agent) }
                },
                cycleFocus: {
                    Task { await model.cycleAgentFocus() }
                },
                setExecutionMode: { agent, mode in
                    Task { await model.setAgentExecutionMode(agent: agent, mode: mode) }
                },
                setPermissionLevel: { agent, level in
                    Task { await model.setAgentPermissionLevel(agent: agent, level: level) }
                }
            )
            AttachmentSummary(
                attachment: model.activeAttachment,
                eventStreamState: model.eventStreamState,
                lastEventID: model.lastEventID,
                lastHeartbeatAt: model.lastHeartbeatAt
            )

            Spacer(minLength: 0)
        }
        .padding(20)
        .background(ArrobaPalette.background)
    }
}

private struct TerminalField: View {
    let title: String
    @Binding var text: String
    let prompt: String

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text(title.uppercased())
                .font(.system(.caption, design: .monospaced, weight: .semibold))
                .foregroundStyle(ArrobaPalette.orange)
            TextField(prompt, text: $text)
                .arrobaTerminalInput()
                .font(.system(.callout, design: .monospaced))
                .padding(.horizontal, 12)
                .padding(.vertical, 10)
                .foregroundStyle(.white)
                .background(ArrobaPalette.field)
                .clipShape(.rect(cornerRadius: 8))
                .overlay(
                    RoundedRectangle(cornerRadius: 8)
                        .stroke(ArrobaPalette.border, lineWidth: 1)
                )
                .accessibilityLabel(title)
        }
    }
}

private struct SessionRow: View {
    let session: RuntimeSession
    let selected: Bool
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            HStack(alignment: .center, spacing: 12) {
                Text(selected ? ">" : " ")
                    .font(.system(.body, design: .monospaced, weight: .bold))
                    .foregroundStyle(ArrobaPalette.orange)
                    .frame(width: 16)
                VStack(alignment: .leading, spacing: 4) {
                    Text(session.shortDisplayID)
                        .font(.system(.body, design: .monospaced, weight: .semibold))
                        .foregroundStyle(.white)
                    Text(session.worktreeID)
                        .font(.system(.caption, design: .monospaced))
                        .foregroundStyle(ArrobaPalette.muted)
                        .lineLimit(1)
                        .truncationMode(.middle)
                }
                Spacer()
                VStack(alignment: .trailing, spacing: 4) {
                    Text(session.status.uppercased())
                        .font(.system(.caption, design: .monospaced, weight: .bold))
                        .foregroundStyle(selected ? ArrobaPalette.orange : ArrobaPalette.muted)
                    Text(session.activeAgentCountText)
                        .font(.system(.caption2, design: .monospaced))
                        .foregroundStyle(ArrobaPalette.muted)
                }
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 12)
            .background(selected ? ArrobaPalette.selected : ArrobaPalette.panel)
        }
        .buttonStyle(.plain)
        .accessibilityIdentifier("session-\(session.shortDisplayID)")
    }
}

private struct ProviderCatalogSummary: View {
    let catalog: ProviderCatalog?
    let authStatuses: [String: ProviderAuthStatus]
    let mcpServers: [ArrobaMcpServerConfig]
    let skills: [ArrobaSkillMetadata]

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text("Capabilities")
                .font(.system(.caption, design: .monospaced, weight: .semibold))
                .foregroundStyle(ArrobaPalette.orange)
            if let catalog {
                SummaryLine(label: "connected", value: catalog.connected.joined(separator: ", "))
                SummaryLine(label: "catalog", value: "\(catalog.all.count) provider\(catalog.all.count == 1 ? "" : "s")")
                if !authStatuses.isEmpty {
                    SummaryLine(
                        label: "auth",
                        value: authStatuses.values
                            .sorted { $0.provider < $1.provider }
                            .map { "\($0.provider)=\($0.authState)" }
                            .joined(separator: ", ")
                    )
                }
            } else {
                Text("Run /provider list to load provider inventory.")
                    .font(.system(.callout, design: .monospaced))
                    .foregroundStyle(ArrobaPalette.muted)
            }
            if !mcpServers.isEmpty {
                SummaryLine(label: "mcps", value: "\(mcpServers.count)")
            }
            if !skills.isEmpty {
                SummaryLine(label: "skills", value: "\(skills.count)")
            }
        }
    }
}

private struct AgentFocusPanel: View {
    let session: RuntimeSession?
    let isWorking: Bool
    let spawnAgent: () -> Void
    let focusAgent: (AgentInstance) -> Void
    let destroyAgent: (AgentInstance) -> Void
    let cycleFocus: () -> Void
    let setExecutionMode: (AgentInstance, AgentExecutionMode?) -> Void
    let setPermissionLevel: (AgentInstance, AgentPermissionLevel?) -> Void

    @State private var pendingDestroyAgent: AgentInstance?

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack {
                Text("Agents")
                    .font(.system(.caption, design: .monospaced, weight: .semibold))
                    .foregroundStyle(ArrobaPalette.orange)
                Spacer()
                Button(action: spawnAgent) {
                    Image(systemName: "plus")
                        .font(.system(.caption, weight: .bold))
                }
                .buttonStyle(AgentIconButtonStyle(primary: true))
                .disabled(isWorking || session == nil)
                .accessibilityLabel("Spawn agent")
                .accessibilityIdentifier("agent-spawn")

                Button(action: cycleFocus) {
                    Image(systemName: "arrow.triangle.2.circlepath")
                        .font(.system(.caption, weight: .bold))
                }
                .buttonStyle(AgentIconButtonStyle(primary: true))
                .accessibilityLabel("Cycle agent focus")
                .accessibilityIdentifier("agent-cycle-focus")
                .disabled(isWorking || session == nil || session?.agents.isEmpty == true)
            }

            if let session, !session.agents.isEmpty {
                VStack(spacing: 1) {
                    ForEach(session.agents) { agent in
                        AgentRow(
                            agent: agent,
                            isFocused: session.focusedAgentID == agent.id,
                            isWorking: isWorking,
                            focus: {
                                focusAgent(agent)
                            },
                            requestDestroy: {
                                pendingDestroyAgent = agent
                            },
                            setExecutionMode: { mode in
                                setExecutionMode(agent, mode)
                            },
                            setPermissionLevel: { level in
                                setPermissionLevel(agent, level)
                            }
                        )
                    }
                }
                .background(ArrobaPalette.border)
                .clipShape(.rect(cornerRadius: 8))
            } else {
                Text("No agents reported for the selected session yet.")
                    .font(.system(.callout, design: .monospaced))
                    .foregroundStyle(ArrobaPalette.muted)
            }
        }
        .accessibilityIdentifier("agent-focus-panel")
        .confirmationDialog(
            "Destroy agent?",
            isPresented: Binding(
                get: { pendingDestroyAgent != nil },
                set: { isPresented in
                    if !isPresented {
                        pendingDestroyAgent = nil
                    }
                }
            ),
            titleVisibility: .visible
        ) {
            if let pendingDestroyAgent {
                Button("Destroy \(pendingDestroyAgent.displayName)", role: .destructive) {
                    destroyAgent(pendingDestroyAgent)
                    self.pendingDestroyAgent = nil
                }
            }
            Button("Cancel", role: .cancel) {
                pendingDestroyAgent = nil
            }
        }
    }
}

private struct AgentRow: View {
    let agent: AgentInstance
    let isFocused: Bool
    let isWorking: Bool
    let focus: () -> Void
    let requestDestroy: () -> Void
    let setExecutionMode: (AgentExecutionMode?) -> Void
    let setPermissionLevel: (AgentPermissionLevel?) -> Void

    var body: some View {
        HStack(alignment: .center, spacing: 8) {
            Button(action: focus) {
                HStack(alignment: .center, spacing: 10) {
                    Text(isFocused ? "*" : " ")
                        .font(.system(.body, design: .monospaced, weight: .bold))
                        .foregroundStyle(ArrobaPalette.orange)
                        .frame(width: 14)
                    VStack(alignment: .leading, spacing: 3) {
                        Text(agent.displayName)
                            .font(.system(.caption, design: .monospaced, weight: .semibold))
                            .foregroundStyle(.white)
                        Text(agent.providerModelText)
                            .font(.system(.caption2, design: .monospaced))
                            .foregroundStyle(ArrobaPalette.muted)
                            .lineLimit(1)
                        Text("mode \(agent.executionModeText) / perms \(agent.permissionLevelText)")
                            .font(.system(.caption2, design: .monospaced))
                            .foregroundStyle(ArrobaPalette.muted)
                            .lineLimit(1)
                    }
                    Spacer()
                    VStack(alignment: .trailing, spacing: 3) {
                        Text(agent.state.uppercased())
                            .font(.system(.caption2, design: .monospaced, weight: .bold))
                            .foregroundStyle(isFocused ? ArrobaPalette.orange : ArrobaPalette.muted)
                        if agent.isProcessing {
                            Text("BUSY")
                                .font(.system(.caption2, design: .monospaced, weight: .bold))
                                .foregroundStyle(ArrobaPalette.red)
                        }
                    }
                }
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .disabled(isWorking || isFocused)

            Menu {
                Menu("Mode") {
                    Button("Build") { setExecutionMode(.build) }
                    Button("Plan") { setExecutionMode(.plan) }
                    Button("Inherit") { setExecutionMode(nil) }
                }
                Menu("Permissions") {
                    Button("Required") { setPermissionLevel(.required) }
                    Button("Yolo") { setPermissionLevel(.yolo) }
                    Button("Inherit") { setPermissionLevel(nil) }
                }
                Button("Destroy", role: .destructive, action: requestDestroy)
            } label: {
                Image(systemName: "ellipsis")
                    .font(.system(.caption, weight: .bold))
                    .frame(width: 28, height: 28)
            }
            .buttonStyle(AgentIconButtonStyle())
            .disabled(isWorking)
            .accessibilityLabel("Agent actions")
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 10)
        .background(isFocused ? ArrobaPalette.selected : ArrobaPalette.field)
        .accessibilityLabel("\(agent.displayName), \(agent.providerModelText)")
        .accessibilityValue(isFocused ? "focused" : agent.state)
        .accessibilityIdentifier("agent-focus-\(agent.id)")
    }
}

private struct SelectedSessionSummary: View {
    let session: RuntimeSession?

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text("Selected")
                .font(.system(.caption, design: .monospaced, weight: .semibold))
                .foregroundStyle(ArrobaPalette.orange)
            if let session {
                SummaryLine(label: "session", value: session.shortDisplayID)
                SummaryLine(label: "status", value: session.status)
                SummaryLine(label: "agents", value: session.activeAgentCountText)
                SummaryLine(label: "prompt", value: session.focusedPromptActivityText)
                if !session.activeInteractions.isEmpty {
                    SummaryLine(label: "interactions", value: "\(session.activeInteractions.count) pending")
                }
                SummaryLine(label: "workspace", value: session.workspaceID)
                SummaryLine(label: "worktree", value: session.worktreeID)
            } else {
                Text("No session selected.")
                    .font(.system(.callout, design: .monospaced))
                    .foregroundStyle(ArrobaPalette.muted)
            }
        }
    }
}

private struct AttachmentSummary: View {
    let attachment: RuntimeAttachment?
    let eventStreamState: EventStreamState
    let lastEventID: Int64?
    let lastHeartbeatAt: Date?

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text("Attachment")
                .font(.system(.caption, design: .monospaced, weight: .semibold))
                .foregroundStyle(ArrobaPalette.orange)
            if let attachment {
                SummaryLine(label: "attachment", value: String(attachment.id.prefix(12)))
                SummaryLine(label: "stream", value: eventStreamState.label)
                if let lastEventID {
                    SummaryLine(label: "last event", value: "\(lastEventID)")
                }
                if let lastHeartbeatAt {
                    SummaryLine(label: "heartbeat", value: lastHeartbeatAt.formatted(date: .omitted, time: .standard))
                }
            } else {
                Text("Attach to the selected session to receive live kernel events.")
                    .font(.system(.callout, design: .monospaced))
                    .foregroundStyle(ArrobaPalette.muted)
            }
        }
        .padding(.top, 6)
    }
}

private struct SummaryLine: View {
    let label: String
    let value: String

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(label.uppercased())
                .font(.system(.caption2, design: .monospaced, weight: .bold))
                .foregroundStyle(ArrobaPalette.muted)
            Text(value)
                .font(.system(.caption, design: .monospaced))
                .foregroundStyle(.white)
                .lineLimit(2)
                .truncationMode(.middle)
        }
    }
}

private struct EmptyStateRow: View {
    var body: some View {
        HStack(spacing: 12) {
            Text(">")
                .font(.system(.body, design: .monospaced, weight: .bold))
                .foregroundStyle(ArrobaPalette.orange)
            Text("No sessions found. Create one against an explicit workspace/worktree target.")
                .font(.system(.callout, design: .monospaced))
                .foregroundStyle(ArrobaPalette.muted)
        }
        .padding(14)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(ArrobaPalette.field)
        .clipShape(.rect(cornerRadius: 8))
    }
}

private struct StatusPill: View {
    let state: ConnectionState

    var body: some View {
        Text(state.label)
            .font(.system(.caption, design: .monospaced, weight: .bold))
            .padding(.horizontal, 10)
            .padding(.vertical, 6)
            .foregroundStyle(state == .failed ? ArrobaPalette.red : ArrobaPalette.orange)
            .background(ArrobaPalette.field)
            .clipShape(.rect(cornerRadius: 6))
    }
}

private struct GlobalFooter: View {
    let state: ConnectionState
    let message: String

    var body: some View {
        HStack(spacing: 12) {
            Text(state.label)
                .font(.system(.caption, design: .monospaced, weight: .bold))
                .foregroundStyle(state == .failed ? ArrobaPalette.red : ArrobaPalette.orange)
            Text(message)
                .font(.system(.caption, design: .monospaced))
                .foregroundStyle(ArrobaPalette.muted)
                .lineLimit(1)
            Spacer()
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 10)
        .background(ArrobaPalette.background)
        .accessibilityIdentifier("global-footer")
    }
}

private struct ArrobaCommandButtonStyle: ButtonStyle {
    var primary = false

    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .font(.system(.callout, design: .monospaced, weight: .semibold))
            .padding(.horizontal, 14)
            .padding(.vertical, 10)
            .foregroundStyle(primary ? ArrobaPalette.background : .white)
            .background(primary ? ArrobaPalette.orange : ArrobaPalette.field)
            .clipShape(.rect(cornerRadius: 8))
            .opacity(configuration.isPressed ? 0.7 : 1)
    }
}

private struct AgentIconButtonStyle: ButtonStyle {
    var primary = false

    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .frame(width: 30, height: 30)
            .foregroundStyle(primary ? ArrobaPalette.background : .white)
            .background(primary ? ArrobaPalette.orange : ArrobaPalette.field)
            .clipShape(.rect(cornerRadius: 6))
            .opacity(configuration.isPressed ? 0.7 : 1)
    }
}

private extension ConnectionState {
    var isWorking: Bool {
        if case .working = self { return true }
        return false
    }
}

private enum ArrobaPalette {
    static let background = Color(red: 0.035, green: 0.039, blue: 0.043)
    static let panel = Color(red: 0.063, green: 0.067, blue: 0.070)
    static let field = Color(red: 0.098, green: 0.105, blue: 0.110)
    static let selected = Color(red: 0.142, green: 0.108, blue: 0.070)
    static let border = Color(red: 0.180, green: 0.190, blue: 0.190)
    static let muted = Color(red: 0.620, green: 0.650, blue: 0.640)
    static let orange = Color(red: 0.960, green: 0.445, blue: 0.160)
    static let red = Color(red: 0.980, green: 0.280, blue: 0.240)
}

private extension View {
    @ViewBuilder
    func arrobaTerminalInput() -> some View {
        #if os(iOS)
        self
            .textInputAutocapitalization(.never)
            .autocorrectionDisabled()
        #else
        self
        #endif
    }

    @ViewBuilder
    func arrobaTerminalEditor() -> some View {
        #if os(iOS)
        self
            .scrollContentBackground(.hidden)
            .textInputAutocapitalization(.sentences)
            .autocorrectionDisabled(false)
        #else
        self
        #endif
    }
}

#Preview {
    ArrobaRootView(model: ArrobaAppModel())
}

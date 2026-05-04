import SwiftUI

public struct ArrobaRootView: View {
    @Bindable private var model: ArrobaAppModel

    public init(model: ArrobaAppModel) {
        self.model = model
    }

    public var body: some View {
        GeometryReader { proxy in
            let isWide = proxy.size.width >= 760
            HStack(spacing: 0) {
                if isWide {
                    TerminalRail(model: model)
                        .frame(width: 72)
                    Divider()
                        .overlay(ArrobaPalette.border)
                }
                TerminalStage(model: model, isWide: isWide)
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

private struct TerminalStage: View {
    @Bindable var model: ArrobaAppModel
    let isWide: Bool

    private var hasAttachment: Bool {
        model.activeAttachment != nil
    }

    var body: some View {
        VStack(spacing: 0) {
            TerminalHeader(model: model, isWide: isWide)
            if hasAttachment {
                FreeformSurface(model: model, isWide: isWide)
            } else {
                WaitingRoomSurface(model: model, isWide: isWide)
            }
            PromptStrip(model: model, mode: hasAttachment ? .freeform : .waiting)
            GlobalFooter(
                state: model.connectionState,
                streamState: model.eventStreamState,
                session: model.selectedSession,
                attachment: model.activeAttachment,
                message: model.statusMessage
            )
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(ArrobaPalette.panel)
    }
}

private struct TerminalHeader: View {
    @Bindable var model: ArrobaAppModel
    let isWide: Bool

    var body: some View {
        HStack(alignment: .center, spacing: 14) {
            Text("@")
                .font(.system(size: isWide ? 36 : 30, weight: .heavy, design: .monospaced))
                .foregroundStyle(ArrobaPalette.orange)
                .frame(width: isWide ? 44 : 34, alignment: .leading)
            VStack(alignment: .leading, spacing: 2) {
                Text("ARROBA")
                    .font(.system(.headline, design: .monospaced, weight: .bold))
                    .foregroundStyle(.white)
                Text(headerDetail)
                    .font(.system(.caption, design: .monospaced))
                    .foregroundStyle(ArrobaPalette.muted)
                    .lineLimit(1)
                    .truncationMode(.middle)
            }
            Spacer(minLength: 10)
            StatusPill(label: model.connectionState.label, tone: model.connectionState == .failed ? .danger : .accent)
        }
        .padding(.horizontal, isWide ? 18 : 14)
        .padding(.vertical, 12)
        .background(ArrobaPalette.background)
        .overlay(alignment: .bottom) {
            Rectangle()
                .fill(ArrobaPalette.border)
                .frame(height: 1)
        }
        .accessibilityElement(children: .combine)
        .accessibilityLabel("Arroba terminal")
    }

    private var headerDetail: String {
        if let session = model.selectedSession {
            return "\(session.shortDisplayID) / \(session.focusedAgent?.displayName ?? "no agent")"
        }
        return model.kernelURLText
    }
}

private struct TerminalRail: View {
    @Bindable var model: ArrobaAppModel

    var body: some View {
        VStack(spacing: 14) {
            Text("@")
                .font(.system(.title2, design: .monospaced, weight: .heavy))
                .foregroundStyle(ArrobaPalette.orange)
                .padding(.top, 16)
            RailButton(systemName: "terminal", label: "Waiting room") {
                Task { await model.detachActiveSession() }
            }
            .disabled(model.activeAttachment == nil || model.connectionState.isWorking)
            RailButton(systemName: "arrow.triangle.2.circlepath", label: "Cycle agent") {
                Task { await model.cycleAgentFocus() }
            }
            .disabled(model.selectedSession?.agents.isEmpty != false || model.connectionState.isWorking)
            RailButton(systemName: "plus", label: "Spawn agent") {
                Task { await model.spawnAgent() }
            }
            .disabled(model.selectedSession == nil || model.connectionState.isWorking)
            RailButton(systemName: "stop.fill", label: "Stop") {
                Task { await model.cancelActivePrompt() }
            }
            .disabled(model.activeAttachment == nil || model.connectionState.isWorking)
            Spacer(minLength: 0)
        }
        .padding(.horizontal, 10)
        .background(ArrobaPalette.background)
    }
}

private struct RailButton: View {
    let systemName: String
    let label: String
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            Image(systemName: systemName)
                .font(.system(.body, weight: .semibold))
                .frame(width: 42, height: 42)
        }
        .buttonStyle(.plain)
        .foregroundStyle(.white)
        .background(ArrobaPalette.field)
        .clipShape(.rect(cornerRadius: 8))
        .accessibilityLabel(label)
    }
}

private struct WaitingRoomSurface: View {
    @Bindable var model: ArrobaAppModel
    let isWide: Bool
    @FocusState private var focusedField: Field?

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 18) {
                logoBlock
                if isWide {
                    HStack(alignment: .top, spacing: 24) {
                        VStack(alignment: .leading, spacing: 18) {
                            settingsBlock
                            waitingRows
                        }
                        .frame(maxWidth: .infinity, alignment: .topLeading)

                        VStack(alignment: .leading, spacing: 18) {
                            InteractionPromptPanel(model: model)
                            RuntimeInventory(model: model)
                            Spacer(minLength: 0)
                        }
                        .frame(width: 320, alignment: .topLeading)
                    }
                } else {
                    settingsBlock
                    waitingRows
                    InteractionPromptPanel(model: model)
                    RuntimeInventory(model: model)
                }
            }
            .frame(maxWidth: isWide ? 1120 : .infinity, alignment: .leading)
            .padding(.horizontal, isWide ? 28 : 16)
            .padding(.vertical, isWide ? 28 : 18)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(ArrobaPalette.panel)
        .accessibilityIdentifier("waiting-room")
    }

    private var logoBlock: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text("@ ARROBA")
                .font(.system(isWide ? .title2 : .title3, design: .monospaced, weight: .heavy))
                .foregroundStyle(ArrobaPalette.orange)
                .textSelection(.enabled)
            Text(model.statusMessage)
                .font(.system(.caption, design: .monospaced))
                .foregroundStyle(model.connectionState == .failed ? ArrobaPalette.red : ArrobaPalette.muted)
                .textSelection(.enabled)
        }
    }

    private var settingsBlock: some View {
        VStack(spacing: 1) {
            TerminalField(title: "kernel", text: $model.kernelURLText, prompt: "ws://127.0.0.1:43118/kernel")
                .focused($focusedField, equals: .kernel)
            TerminalField(title: "workspace", text: $model.workspacePath, prompt: "/path/to/repo on kernel host")
                .focused($focusedField, equals: .workspace)
            TerminalField(title: "worktree", text: $model.worktreePath, prompt: "/path/to/worktree on kernel host")
                .focused($focusedField, equals: .worktree)
        }
        .clipShape(.rect(cornerRadius: 8))
        .overlay(
            RoundedRectangle(cornerRadius: 8)
                .stroke(ArrobaPalette.border, lineWidth: 1)
        )
    }

    private var waitingRows: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text("Waiting room")
                .terminalSectionLabel()
            VStack(spacing: 1) {
                WaitingActionRow(marker: ">", label: "Start new session", value: "\(model.selectedProviderID) \(model.selectedModelID) \(model.selectedVariantID)") {
                    Task {
                        await model.createSession()
                        if model.connectionState == .connected {
                            await model.attachSelectedSession()
                        }
                    }
                }
                .disabled(model.connectionState.isWorking)

                WaitingActionRow(marker: " ", label: "Refresh sessions", value: model.kernelURLText) {
                    Task { await model.refreshSessions() }
                }
                .disabled(model.connectionState.isWorking)

                ForEach(model.sessions) { session in
                    WaitingSessionRow(
                        session: session,
                        selected: model.selectedSessionID == session.id,
                        attach: {
                            model.selectSession(session)
                            Task { await model.attachSelectedSession() }
                        },
                        select: {
                            model.selectSession(session)
                        }
                    )
                    .disabled(model.connectionState.isWorking)
                }

                if model.sessions.isEmpty {
                    StaticTerminalRow(marker: " ", label: "No sessions yet", value: "Create one from the configured workspace")
                }
            }
            .background(ArrobaPalette.border)
            .clipShape(.rect(cornerRadius: 8))
        }
    }

    private enum Field {
        case kernel
        case workspace
        case worktree
    }
}

private struct FreeformSurface: View {
    @Bindable var model: ArrobaAppModel
    let isWide: Bool

    private var visibleAgents: [AgentInstance] {
        guard let session = model.selectedSession else { return [] }
        if model.responseLayout == .individual {
            if let focused = session.focusedAgent {
                return [focused]
            }
            return Array(session.agents.prefix(1))
        }
        return session.agents.prefixPreservingFocusedAgent(
            focusedAgentID: session.focusedAgentID,
            limit: isWide ? 6 : 3
        )
    }

    var body: some View {
        Group {
            if visibleAgents.isEmpty {
                VStack(spacing: 12) {
                    Text("@")
                        .font(.system(size: 64, weight: .heavy, design: .monospaced))
                        .foregroundStyle(ArrobaPalette.orange)
                    Text("No agents reported yet")
                        .font(.system(.callout, design: .monospaced))
                        .foregroundStyle(ArrobaPalette.muted)
                    Button {
                        Task { await model.spawnAgent() }
                    } label: {
                        Label("Spawn agent", systemImage: "plus")
                    }
                    .buttonStyle(ArrobaCommandButtonStyle(primary: true))
                    .disabled(model.connectionState.isWorking)
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else if model.responseLayout == .split, isWide {
                AgentGrid(model: model, agents: visibleAgents)
            } else {
                AgentPager(model: model, agents: visibleAgents)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(ArrobaPalette.panel)
        .accessibilityIdentifier("freeform-surface")
    }
}

private struct AgentGrid: View {
    @Bindable var model: ArrobaAppModel
    let agents: [AgentInstance]

    var body: some View {
        GeometryReader { proxy in
            let columns = [GridItem(.flexible(), spacing: 1), GridItem(.flexible(), spacing: 1)]
            ScrollView {
                LazyVGrid(columns: columns, spacing: 1) {
                    ForEach(agents) { agent in
                        AgentPane(
                            model: model,
                            agent: agent,
                            focused: model.selectedSession?.focusedAgentID == agent.id
                        )
                        .frame(height: max(260, (proxy.size.height - 1) / 2))
                    }
                }
            }
        }
    }
}

private struct AgentPager: View {
    @Bindable var model: ArrobaAppModel
    let agents: [AgentInstance]

    var body: some View {
        TabView(selection: Binding(
            get: { model.selectedSession?.focusedAgentID ?? agents.first?.id ?? "" },
            set: { nextID in
                guard let agent = agents.first(where: { $0.id == nextID }) else { return }
                Task { await model.focusAgent(agent) }
            }
        )) {
            ForEach(agents) { agent in
                AgentPane(
                    model: model,
                    agent: agent,
                    focused: model.selectedSession?.focusedAgentID == agent.id
                )
                .tag(agent.id)
            }
        }
        #if os(iOS)
        .tabViewStyle(.page(indexDisplayMode: agents.count > 1 ? .automatic : .never))
        #endif
    }
}

private struct AgentPane: View {
    @Bindable var model: ArrobaAppModel
    let agent: AgentInstance
    let focused: Bool
    @State private var pendingDestroy = false

    var body: some View {
        VStack(spacing: 0) {
            TranscriptSurface(entries: model.transcriptEntries(for: agent), agent: agent)
            AgentPaneFooter(
                agent: agent,
                focused: focused,
                isWorking: model.connectionState.isWorking,
                focus: { Task { await model.focusAgent(agent) } },
                stop: { Task { await model.cancelActivePrompt() } },
                destroy: { pendingDestroy = true }
            )
        }
        .background(ArrobaPalette.panel)
        .overlay(
            Rectangle()
                .stroke(focused ? ArrobaPalette.orange.opacity(0.65) : ArrobaPalette.border, lineWidth: focused ? 1.5 : 1)
        )
        .confirmationDialog("Destroy agent?", isPresented: $pendingDestroy, titleVisibility: .visible) {
            Button("Destroy \(agent.displayName)", role: .destructive) {
                Task { await model.destroyAgent(agent) }
            }
            Button("Cancel", role: .cancel) {}
        }
        .accessibilityIdentifier("agent-pane-\(agent.id)")
    }
}

private struct TranscriptSurface: View {
    let entries: [TranscriptEntry]
    let agent: AgentInstance

    var body: some View {
        ScrollViewReader { proxy in
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 10) {
                    if entries.isEmpty {
                        VStack(alignment: .leading, spacing: 8) {
                            Text("@")
                                .font(.system(size: 48, weight: .heavy, design: .monospaced))
                                .foregroundStyle(ArrobaPalette.orange)
                            Text("Type your first prompt below")
                                .font(.system(.callout, design: .monospaced))
                                .foregroundStyle(ArrobaPalette.muted)
                        }
                        .padding(18)
                        .frame(maxWidth: .infinity, alignment: .leading)
                    } else {
                        ForEach(entries) { entry in
                            TranscriptRow(entry: entry)
                                .id(entry.id)
                        }
                    }
                }
                .padding(14)
            }
            .onChange(of: entries.last?.id) {
                guard let id = entries.last?.id else { return }
                withAnimation(.easeOut(duration: 0.2)) {
                    proxy.scrollTo(id, anchor: .bottom)
                }
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}

private struct AgentPaneFooter: View {
    let agent: AgentInstance
    let focused: Bool
    let isWorking: Bool
    let focus: () -> Void
    let stop: () -> Void
    let destroy: () -> Void

    var body: some View {
        HStack(spacing: 10) {
            Button(action: focus) {
                HStack(spacing: 8) {
                    Text(agent.isProcessing ? "BUSY" : agent.state.uppercased())
                        .font(.system(.caption2, design: .monospaced, weight: .bold))
                        .foregroundStyle(agent.isProcessing ? ArrobaPalette.red : ArrobaPalette.orange)
                    Text(agent.displayName)
                        .font(.system(.caption, design: .monospaced, weight: .semibold))
                        .foregroundStyle(.white)
                    Text(agent.providerModelText)
                        .font(.system(.caption, design: .monospaced))
                        .foregroundStyle(ArrobaPalette.muted)
                        .lineLimit(1)
                    if let effort = agent.effort {
                        Text(effort)
                            .font(.system(.caption, design: .monospaced))
                            .foregroundStyle(ArrobaPalette.muted)
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
            }
            .buttonStyle(.plain)
            .disabled(focused || isWorking)

            if agent.isProcessing {
                Button(action: stop) {
                    Image(systemName: "stop.fill")
                }
                .buttonStyle(AgentIconButtonStyle())
                .disabled(isWorking)
                .accessibilityLabel("Stop agent")
            }

            Menu {
                Button("Focus", action: focus)
                Button("Destroy", role: .destructive, action: destroy)
            } label: {
                Image(systemName: "ellipsis")
            }
            .buttonStyle(AgentIconButtonStyle())
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 10)
        .background(focused ? ArrobaPalette.selected : ArrobaPalette.background)
        .overlay(alignment: .top) {
            Rectangle()
                .fill(ArrobaPalette.border)
                .frame(height: 1)
        }
    }
}

private struct PromptStrip: View {
    enum Mode {
        case waiting
        case freeform
    }

    @Bindable var model: ArrobaAppModel
    let mode: Mode

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            if !model.commandCenterItems.isEmpty {
                CommandCenterPanel(items: model.commandCenterItems) { item in
                    Task { await model.executeCommandCenterItem(item) }
                }
            }
            HStack(alignment: .bottom, spacing: 10) {
                TextEditor(text: $model.promptDraft)
                    .arrobaTerminalEditor()
                    .font(.system(.callout, design: .monospaced))
                    .foregroundStyle(.white)
                    .frame(minHeight: editorHeight.minimum, maxHeight: editorHeight.maximum)
                    .padding(.horizontal, 10)
                    .padding(.vertical, 8)
                    .background(ArrobaPalette.field)
                    .clipShape(.rect(cornerRadius: 8))
                    .overlay(
                        RoundedRectangle(cornerRadius: 8)
                            .stroke(ArrobaPalette.border, lineWidth: 1)
                    )
                    .accessibilityLabel("Prompt")
                    .accessibilityIdentifier("prompt-composer")

                VStack(spacing: 8) {
                    Button {
                        Task { await model.submitPrompt() }
                    } label: {
                        Image(systemName: "arrow.up")
                    }
                    .buttonStyle(AgentIconButtonStyle(primary: true))
                    .disabled(sendDisabled)
                    .accessibilityLabel("Send")
                    .accessibilityIdentifier("prompt-send")

                    Button {
                        Task { await model.cancelActivePrompt() }
                    } label: {
                        Image(systemName: "stop.fill")
                    }
                    .buttonStyle(AgentIconButtonStyle())
                    .disabled(model.activeAttachment == nil || model.connectionState.isWorking)
                    .accessibilityLabel("Stop")
                    .accessibilityIdentifier("prompt-stop")
                }
            }
            HStack(spacing: 12) {
                Text(mode == .waiting ? "WAITING ROOM" : "FREEFORM")
                    .font(.system(.caption2, design: .monospaced, weight: .bold))
                    .foregroundStyle(ArrobaPalette.orange)
                Text(metaText)
                    .font(.system(.caption2, design: .monospaced))
                    .foregroundStyle(ArrobaPalette.muted)
                    .lineLimit(1)
                Spacer(minLength: 8)
                if mode == .freeform {
                    Button("Next agent") {
                        Task { await model.cycleAgentFocus() }
                    }
                    .buttonStyle(.plain)
                    .font(.system(.caption2, design: .monospaced, weight: .bold))
                    .foregroundStyle(ArrobaPalette.orange)
                    .disabled(model.connectionState.isWorking)
                    Button("Spawn") {
                        Task { await model.spawnAgent() }
                    }
                    .buttonStyle(.plain)
                    .font(.system(.caption2, design: .monospaced, weight: .bold))
                    .foregroundStyle(ArrobaPalette.orange)
                    .disabled(model.connectionState.isWorking)
                }
            }
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 10)
        .background(ArrobaPalette.background)
        .overlay(alignment: .top) {
            Rectangle()
                .fill(ArrobaPalette.border)
                .frame(height: 1)
        }
    }

    private var sendDisabled: Bool {
        if model.promptDraft.trimmingCharacters(in: .whitespacesAndNewlines).hasPrefix("/") {
            return model.connectionState.isWorking
        }
        return model.connectionState.isWorking || model.activeAttachment == nil
    }

    private var editorHeight: (minimum: CGFloat, maximum: CGFloat) {
        mode == .waiting ? (44, 68) : (52, 96)
    }

    private var metaText: String {
        if mode == .waiting {
            return "\(model.sessions.count) sessions / \(model.selectedProviderID) \(model.selectedModelID) \(model.selectedVariantID)"
        }
        let agent = model.selectedSession?.focusedAgent?.displayName ?? "no agent"
        return "\(agent) / \(model.responseLayout.rawValue) / slash commands"
    }
}

private struct CommandCenterPanel: View {
    let items: [CommandCenterItem]
    let select: (CommandCenterItem) -> Void

    var body: some View {
        ScrollView {
            VStack(spacing: 1) {
                ForEach(items) { item in
                    Button {
                        select(item)
                    } label: {
                        HStack(alignment: .firstTextBaseline, spacing: 10) {
                            Text(item.label)
                                .font(.system(.caption, design: .monospaced, weight: .bold))
                                .foregroundStyle(ArrobaPalette.orange)
                                .frame(minWidth: 82, alignment: .leading)
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
        }
        .frame(maxHeight: 248)
        .background(ArrobaPalette.border)
        .clipShape(.rect(cornerRadius: 8))
        .accessibilityIdentifier("command-center")
    }
}

private struct RuntimeInventory: View {
    @Bindable var model: ArrobaAppModel

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("Runtime")
                .terminalSectionLabel()
            HStack(spacing: 8) {
                Button {
                    Task { await model.refreshProviderCatalog() }
                } label: {
                    Label("Providers", systemImage: "server.rack")
                }
                .buttonStyle(ArrobaCommandButtonStyle())
                .disabled(model.connectionState.isWorking)

                Button {
                    Task { await model.refreshMcpServers() }
                } label: {
                    Label("MCP", systemImage: "shippingbox")
                }
                .buttonStyle(ArrobaCommandButtonStyle())
                .disabled(model.connectionState.isWorking)

                Button {
                    Task { await model.refreshSkills() }
                } label: {
                    Label("Skills", systemImage: "wand.and.stars")
                }
                .buttonStyle(ArrobaCommandButtonStyle())
                .disabled(model.connectionState.isWorking)
            }
            VStack(alignment: .leading, spacing: 6) {
                SummaryLine(label: "provider", value: "\(model.selectedProviderID) / \(model.selectedModelID) / \(model.selectedVariantID)")
                SummaryLine(label: "catalog", value: model.providerCatalog.map { "\($0.connected.joined(separator: ", ")) / \($0.all.count) providers" } ?? "not loaded")
                SummaryLine(label: "mcp", value: "\(model.mcpServers.count)")
                SummaryLine(label: "skills", value: "\(model.skills.count)")
            }
        }
    }
}

private struct InteractionPromptPanel: View {
    @Bindable var model: ArrobaAppModel

    var body: some View {
        let interactions = model.selectedSession?.activeInteractions ?? []
        if !interactions.isEmpty {
            VStack(alignment: .leading, spacing: 8) {
                Text("Interaction")
                    .terminalSectionLabel()
                VStack(spacing: 1) {
                    ForEach(interactions) { interaction in
                        VStack(alignment: .leading, spacing: 10) {
                            HStack {
                                Text(interaction.levelText)
                                    .font(.system(.caption2, design: .monospaced, weight: .bold))
                                    .foregroundStyle(interaction.level == "critical" ? ArrobaPalette.red : ArrobaPalette.orange)
                                Text(interaction.displayTitle)
                                    .font(.system(.callout, design: .monospaced, weight: .semibold))
                                    .foregroundStyle(.white)
                                Spacer()
                            }
                            Text(interaction.message)
                                .font(.system(.caption, design: .monospaced))
                                .foregroundStyle(ArrobaPalette.muted)
                                .textSelection(.enabled)
                            HStack(spacing: 8) {
                                ForEach(interaction.choices) { choice in
                                    Button(choice.label) {
                                        Task { await model.respondToInteraction(interaction, choice: choice) }
                                    }
                                    .buttonStyle(ArrobaCommandButtonStyle(primary: choice.style == "primary"))
                                    .disabled(model.connectionState.isWorking)
                                }
                            }
                        }
                        .padding(12)
                        .background(ArrobaPalette.field)
                    }
                }
                .background(ArrobaPalette.border)
                .clipShape(.rect(cornerRadius: 8))
            }
        }
    }
}

private struct WaitingActionRow: View {
    let marker: String
    let label: String
    let value: String
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            TerminalRowContent(marker: marker, label: label, value: value, selected: marker == ">")
        }
        .buttonStyle(.plain)
    }
}

private struct WaitingSessionRow: View {
    let session: RuntimeSession
    let selected: Bool
    let attach: () -> Void
    let select: () -> Void

    var body: some View {
        Button(action: attach) {
            TerminalRowContent(
                marker: selected ? ">" : " ",
                label: session.shortDisplayID,
                value: "\(session.status) / \(session.activeAgentCountText) / \(session.worktreeID)",
                selected: selected
            )
        }
        .buttonStyle(.plain)
        .simultaneousGesture(LongPressGesture().onEnded { _ in select() })
        .accessibilityIdentifier("session-\(session.shortDisplayID)")
    }
}

private struct StaticTerminalRow: View {
    let marker: String
    let label: String
    let value: String

    var body: some View {
        TerminalRowContent(marker: marker, label: label, value: value, selected: false)
    }
}

private struct TerminalRowContent: View {
    let marker: String
    let label: String
    let value: String
    let selected: Bool

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 10) {
            Text(marker)
                .font(.system(.body, design: .monospaced, weight: .bold))
                .foregroundStyle(ArrobaPalette.orange)
                .frame(width: 16)
            Text(label)
                .font(.system(.caption, design: .monospaced, weight: .semibold))
                .foregroundStyle(selected ? .white : ArrobaPalette.muted)
                .frame(minWidth: 128, alignment: .leading)
            Text(value)
                .font(.system(.caption2, design: .monospaced))
                .foregroundStyle(ArrobaPalette.muted)
                .lineLimit(1)
                .truncationMode(.middle)
            Spacer(minLength: 0)
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 11)
        .background(selected ? ArrobaPalette.selected : ArrobaPalette.field)
    }
}

private struct TerminalField: View {
    let title: String
    @Binding var text: String
    let prompt: String

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 10) {
            Text(title.uppercased())
                .font(.system(.caption, design: .monospaced, weight: .bold))
                .foregroundStyle(ArrobaPalette.orange)
                .frame(width: 86, alignment: .leading)
            TextField(prompt, text: $text)
                .arrobaTerminalInput()
                .font(.system(.caption, design: .monospaced))
                .foregroundStyle(.white)
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 11)
        .background(ArrobaPalette.field)
        .accessibilityLabel(title)
    }
}

private struct TranscriptRow: View {
    let entry: TranscriptEntry

    var body: some View {
        VStack(alignment: .leading, spacing: 5) {
            HStack(spacing: 8) {
                Text(entry.kind.rawValue.uppercased())
                    .font(.system(.caption2, design: .monospaced, weight: .bold))
                    .foregroundStyle(color)
                if let agentID = entry.agentID {
                    Text(agentID)
                        .font(.system(.caption2, design: .monospaced))
                        .foregroundStyle(ArrobaPalette.muted)
                }
                Spacer(minLength: 0)
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

private struct SummaryLine: View {
    let label: String
    let value: String

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 10) {
            Text(label.uppercased())
                .font(.system(.caption2, design: .monospaced, weight: .bold))
                .foregroundStyle(ArrobaPalette.muted)
                .frame(width: 82, alignment: .leading)
            Text(value)
                .font(.system(.caption, design: .monospaced))
                .foregroundStyle(.white)
                .lineLimit(2)
                .truncationMode(.middle)
        }
    }
}

private struct StatusPill: View {
    enum Tone {
        case accent
        case danger
        case muted
    }

    let label: String
    let tone: Tone

    var body: some View {
        Text(label)
            .font(.system(.caption2, design: .monospaced, weight: .bold))
            .padding(.horizontal, 9)
            .padding(.vertical, 6)
            .foregroundStyle(color)
            .background(color.opacity(0.12))
            .clipShape(.rect(cornerRadius: 6))
    }

    private var color: Color {
        switch tone {
        case .accent:
            ArrobaPalette.orange
        case .danger:
            ArrobaPalette.red
        case .muted:
            ArrobaPalette.muted
        }
    }
}

private struct GlobalFooter: View {
    let state: ConnectionState
    let streamState: EventStreamState
    let session: RuntimeSession?
    let attachment: RuntimeAttachment?
    let message: String

    var body: some View {
        HStack(spacing: 12) {
            Text(state.label)
                .font(.system(.caption2, design: .monospaced, weight: .bold))
                .foregroundStyle(state == .failed ? ArrobaPalette.red : ArrobaPalette.orange)
            Text(streamState.label)
                .font(.system(.caption2, design: .monospaced, weight: .bold))
                .foregroundStyle(attachment == nil ? ArrobaPalette.muted : ArrobaPalette.orange)
            Text(session?.shortDisplayID ?? "no session")
                .font(.system(.caption2, design: .monospaced))
                .foregroundStyle(ArrobaPalette.muted)
            Text(message)
                .font(.system(.caption2, design: .monospaced))
                .foregroundStyle(ArrobaPalette.muted)
                .lineLimit(1)
            Spacer(minLength: 0)
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 8)
        .background(ArrobaPalette.background)
        .overlay(alignment: .top) {
            Rectangle()
                .fill(ArrobaPalette.border)
                .frame(height: 1)
        }
        .accessibilityIdentifier("global-footer")
    }
}

private struct ArrobaCommandButtonStyle: ButtonStyle {
    var primary = false

    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .font(.system(.caption, design: .monospaced, weight: .semibold))
            .padding(.horizontal, 12)
            .padding(.vertical, 9)
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
            .font(.system(.body, weight: .semibold))
            .frame(width: 34, height: 34)
            .foregroundStyle(primary ? ArrobaPalette.background : .white)
            .background(primary ? ArrobaPalette.orange : ArrobaPalette.field)
            .clipShape(.rect(cornerRadius: 7))
            .opacity(configuration.isPressed ? 0.7 : 1)
    }
}

private extension View {
    func terminalSectionLabel() -> some View {
        self
            .font(.system(.caption, design: .monospaced, weight: .bold))
            .foregroundStyle(ArrobaPalette.orange)
            .textCase(.uppercase)
    }

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
            .textInputAutocapitalization(.never)
            .autocorrectionDisabled()
        #else
        self
        #endif
    }
}

private extension Array where Element == AgentInstance {
    func prefixPreservingFocusedAgent(focusedAgentID: String?, limit: Int) -> [AgentInstance] {
        guard limit > 0 else { return [] }
        var visible = Array(prefix(limit))
        guard let focusedAgentID,
              !visible.contains(where: { $0.id == focusedAgentID }),
              let focused = first(where: { $0.id == focusedAgentID })
        else {
            return visible
        }
        if visible.count >= limit {
            visible[visible.count - 1] = focused
        } else {
            visible.append(focused)
        }
        return visible
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

#Preview {
    ArrobaRootView(model: ArrobaAppModel())
}

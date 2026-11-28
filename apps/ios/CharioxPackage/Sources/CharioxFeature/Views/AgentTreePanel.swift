import SwiftUI

struct AgentTreePanel: View {
    @Bindable var model: CharioxAppModel
    let session: RuntimeSession
    @State private var expanded = false
    @State private var selectedID: String?

    var body: some View {
        let nodes = model.descendantNodes(in: session)
        DisclosureGroup("Sub-agents \(nodes.count)", isExpanded: $expanded) {
            ForEach(nodes) { node in
                Button {
                    selectedID = node.id
                } label: {
                    HStack {
                        Text(node.displayName)
                        Spacer()
                        Text(node.statusText)
                            .foregroundStyle(node.state == .timeLimitEnded ? CharioxPalette.orange : CharioxPalette.muted)
                    }
                    .padding(.leading, CGFloat(min(node.depth, 6)) * 8)
                }
                .buttonStyle(.plain)
                .accessibilityIdentifier("open-sub-agent-\(node.id)")
            }
            ForEach(session.agents.filter { model.agentTreeErrors[$0.id] != nil }) { root in
                Button("Could not load sub-agents. Retry") {
                    Task { await model.refreshAgentTree(sessionID: session.id, rootAgentID: root.id) }
                }
            }
            if let node = nodes.first(where: { $0.id == selectedID }) {
                VStack(alignment: .leading, spacing: 8) {
                    Text(node.displayName).terminalSectionLabel()
                    Text(node.statusText)
                    InteractionPromptPanel(model: model, agentID: node.id)
                }
                .accessibilityIdentifier("sub-agent-pane-\(node.id)")
            }
        }
        .padding(10)
        .task(id: [session.id, model.activeAttachment?.id ?? ""] + session.agents.map(\.id)) {
            for root in session.agents {
                await model.refreshAgentTree(sessionID: session.id, rootAgentID: root.id)
            }
        }
    }
}

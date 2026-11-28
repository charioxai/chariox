import Foundation

@MainActor
extension CharioxAppModel {
    public func refreshAgentTree(sessionID: String, rootAgentID: String) async {
        guard let attachment = activeAttachment, attachment.sessionID == sessionID,
              let session = sessions.first(where: { $0.id == sessionID }),
              session.agents.contains(where: { $0.id == rootAgentID }) else { return }
        do {
            let response = try await client.send(.getAgentTree(sessionID: sessionID, agentID: rootAgentID), to: endpointURL())
            guard case .agentTree(let tree) = response, tree.root == rootAgentID,
                  Set(tree.nodes.map(\.id)).count == tree.nodes.count else {
                throw KernelClientError.unexpectedResponse("Agent tree does not match its Room root")
            }
            guard activeAttachment?.id == attachment.id else { return }
            if agentTreeAttachments[rootAgentID] == attachment.id,
               let current = agentTrees[rootAgentID], current.rev > tree.rev { return }
            agentTreeAttachments[rootAgentID] = attachment.id
            agentTrees[rootAgentID] = tree
            agentTreeErrors.removeValue(forKey: rootAgentID)
        } catch {
            guard activeAttachment?.id == attachment.id else { return }
            // Keep the last authoritative tree; the opened tree offers Retry.
            agentTreeErrors[rootAgentID] = "Could not load sub-agents. Try again."
        }
    }

    public func descendantNodes(in session: RuntimeSession) -> [RuntimeAgentTreeNode] {
        let members = Set(session.agents.map(\.id))
        var seen = Set<String>()
        return session.agents.flatMap { agentTrees[$0.id]?.nodes ?? [] }.filter { node in
            !members.contains(node.id) && (node.roomID == nil || node.roomID == session.id) && seen.insert(node.id).inserted
        }
    }
}

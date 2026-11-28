import Foundation

@MainActor
extension CharioxAppModel {
    public func refreshAgentTree(sessionID: String, rootAgentID: String) async {
        guard let attachment = activeAttachment, attachment.sessionID == sessionID,
              let session = sessions.first(where: { $0.id == sessionID }),
              session.agents.contains(where: { $0.id == rootAgentID }) else { return }
        let generation = agentTreeStreamGeneration
        let readToken = UUID()
        agentTreeReadTokens[rootAgentID] = readToken
        do {
            let response = try await client.send(.getAgentTree(sessionID: sessionID, agentID: rootAgentID), to: endpointURL())
            guard case .agentTree(let tree) = response, tree.root == rootAgentID,
                  Set(tree.nodes.map(\.id)).count == tree.nodes.count else {
                throw KernelClientError.unexpectedResponse("Agent tree does not match its Room root")
            }
            guard activeAttachment?.id == attachment.id,
                  agentTreeStreamGeneration == generation,
                  agentTreeReadTokens[rootAgentID] == readToken else { return }
            if agentTreeAttachments[rootAgentID] == attachment.id,
               agentTreeStreamGenerations[rootAgentID] == generation,
               let current = agentTrees[rootAgentID], current.rev > tree.rev { return }
            agentTreeAttachments[rootAgentID] = attachment.id
            agentTreeStreamGenerations[rootAgentID] = generation
            agentTrees[rootAgentID] = tree
            agentTreeErrors.removeValue(forKey: rootAgentID)
        } catch {
            guard activeAttachment?.id == attachment.id,
                  agentTreeStreamGeneration == generation,
                  agentTreeReadTokens[rootAgentID] == readToken else { return }
            // Keep the last authoritative tree; the opened tree offers Retry.
            agentTreeErrors[rootAgentID] = "Could not load sub-agents. Try again."
        }
    }

    func invalidateAgentTreeReads() {
        agentTreeStreamGeneration &+= 1
        agentTreeBaselinePending = true
    }

    func refreshAgentTreesAfterReconnect(sessionID: String) {
        guard activeAttachment?.sessionID == sessionID,
              let session = sessions.first(where: { $0.id == sessionID }) else { return }
        // A fresh stream can belong to a restarted kernel with lower revisions.
        // Keep its last tree visible while rejecting reads from the old stream.
        invalidateAgentTreeReads()
        agentTreeBaselinePending = false
        for agent in session.agents {
            Task { [weak self] in await self?.refreshAgentTree(sessionID: sessionID, rootAgentID: agent.id) }
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

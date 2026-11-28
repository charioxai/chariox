import Foundation
import Testing
@testable import CharioxFeature

private actor TreeReadLog {
    var requests: [LocalDaemonRequest] = []
    func append(_ request: LocalDaemonRequest) { requests.append(request) }
}
private struct TreeReadClient: KernelClientProtocol {
    let tree: RuntimeAgentTree
    let log: TreeReadLog
    func send(_ request: LocalDaemonRequest, to endpoint: URL) async throws -> LocalDaemonResponse {
        await log.append(request)
        return .agentTree(tree)
    }
    func events(sessionID: String, attachmentID: String, endpoint: URL, resumeFromEventID: Int64?) -> AsyncThrowingStream<KernelEventFrame, Error> {
        AsyncThrowingStream { $0.finish() }
    }
}
private func treeFixture(revision: UInt64 = 1, root: String = "parent") throws -> RuntimeAgentTree {
    try JSONDecoder().decode(RuntimeAgentTree.self, from: JSONSerialization.data(withJSONObject: [
        "root": root, "rev": revision,
        "nodes": [["agent_id": "child", "parent_agent_id": "parent", "alias": "Child", "depth": 1, "state": "time_limit_ended"]],
    ]))
}
private func feedFrame(room: String, interactions: [[String: Any]], eventID: Int64) throws -> KernelEventFrame {
    try JSONDecoder().decode(KernelEventFrame.self, from: JSONSerialization.data(withJSONObject: [
        "type": "event", "event_id": eventID,
        "event": ["event": "runtime_interactions_changed", "session_id": room, "active_interactions": interactions, "machine_approval_receipts": []],
    ]))
}
private func replyQuestion() -> [String: Any] { [
    "id": "question", "agent_id": "parent", "surface": "card", "card_type": "Question", "kind": "choice", "level": "info",
    "message": "Which route?", "choices": [], "custom_choice": ["id": "reply", "label": "Something else"], "requested_at_ms": 1,
] }

@MainActor
@Test func attachedRoomAppliesQuestionAndRetirementFromInteractionEventsAlone() async throws {
    let log = TreeReadLog()
    let model = CharioxAppModel(client: TreeReadClient(tree: try treeFixture(), log: log))
    let parent = AgentInstance.fixture(id: "parent", agentRef: "1", alias: "Parent", state: "Idle")
    let session = RuntimeSession(id: "room", alias: "Room", workspaceID: "workspace", projectID: "project", worktreeID: "work",
        status: "Active", focusedAgentID: parent.id, agents: [parent], workspaceLiveSyncMode: "auto", createdAtMs: 1, lastUsedAtMs: 2, lastActivityAtMs: 3)
    model.selectSession(session)
    model.activeAttachment = RuntimeAttachment(id: "attachment", sessionID: session.id)
    model.subscribedSessionID = session.id
    model.handle(eventFrame: try feedFrame(room: "room", interactions: [replyQuestion()], eventID: 10))
    #expect(model.selectedSession?.activeInteractions.first?.cardType == "Question")
    #expect(model.selectedSession?.activeInteractions.first?.customChoice?.id == "reply")
    #expect(model.selectedSession?.projectID == "project")
    #expect(model.selectedSession?.workspaceLiveSyncMode == "auto")
    #expect(model.selectedSession?.lastActivityAtMs == 3)
    model.handle(eventFrame: try feedFrame(room: "another-room", interactions: [], eventID: 11))
    #expect(model.selectedSessionID == "room")
    #expect(model.selectedSession?.activeInteractions.count == 1)
    model.handle(eventFrame: try feedFrame(room: "room", interactions: [], eventID: 12))
    #expect(model.selectedSession?.activeInteractions.isEmpty == true)
    let requests = await log.requests
    #expect(requests.isEmpty, "interaction events do not require a full Session read")
}

@MainActor
@Test func attachedRoomReadsTreeOnlyChildWithoutInventingAnAgentAndKeepsItsRecoverySubject() async throws {
    let log = TreeReadLog()
    let model = CharioxAppModel(client: TreeReadClient(tree: try treeFixture(), log: log))
    let parent = AgentInstance.fixture(id: "parent", agentRef: "1", alias: "Parent", state: "Idle")
    let session = RuntimeSession.fixture(id: "room", lastUsedAtMs: 2, agents: [parent])
    model.selectSession(session)
    model.activeAttachment = RuntimeAttachment(id: "attachment", sessionID: session.id)
    model.subscribedSessionID = session.id
    await model.refreshAgentTree(sessionID: session.id, rootAgentID: parent.id)
    #expect(model.selectedSession?.agents.map(\.id) == ["parent"])
    #expect(model.descendantNodes(in: session).map(\.id) == ["child"])
    #expect(model.descendantNodes(in: session).first?.statusText == "Time limit ended")
    let recovery: [String: Any] = ["id": "deadline", "kernel_operation_id": "deadline", "related_agent_id": "child", "surface": "pane_status", "kind": "choice", "level": "warning", "message": "Time limit ended", "choices": [["id": "extend", "label": "Extend", "reply": "extend"]], "requested_at_ms": 1]
    model.handle(eventFrame: try feedFrame(room: "room", interactions: [recovery], eventID: 20))
    let shown = try #require(model.selectedSession?.activeInteractions.first)
    #expect(shown.appears(inAgentPane: "child", defaultAgentID: "parent"))
    #expect(!shown.appears(inAgentPane: "parent", defaultAgentID: "parent"))
    #expect(shown.choices.first?.id == "extend")
    let requests = await log.requests
    let request = try #require(requests.first)
    let encoded = try KernelProtocolCodec.encodeRequestFrame(KernelRequestFrame(requestID: "tree", request: request))
    let frame = try #require(JSONSerialization.jsonObject(with: encoded) as? [String: Any])
    let payload = try #require(frame["request"] as? [String: Any])
    #expect((payload["GetAgentTree"] as? [String: String]) == ["session_id": "room", "agent_id": "parent"])
    model.handle(eventFrame: try feedFrame(room: "room", interactions: [], eventID: 21))
    #expect(model.selectedSession?.activeInteractions.isEmpty == true)
}

@MainActor
@Test func attachedRoomKeepsANewerAuthoritativeTreeWhenAnOlderReadReturns() async throws {
    let log = TreeReadLog()
    let model = CharioxAppModel(client: TreeReadClient(tree: try treeFixture(revision: 2), log: log))
    let parent = AgentInstance.fixture(id: "parent", agentRef: "1", alias: "Parent", state: "Idle")
    let session = RuntimeSession.fixture(id: "room", lastUsedAtMs: 2, agents: [parent])
    model.selectSession(session)
    model.activeAttachment = RuntimeAttachment(id: "attachment", sessionID: session.id)
    model.agentTrees[parent.id] = try treeFixture(revision: 3)
    model.agentTreeAttachments[parent.id] = "attachment"
    await model.refreshAgentTree(sessionID: session.id, rootAgentID: parent.id)
    #expect(model.agentTrees[parent.id]?.rev == 3)
    #expect(model.descendantNodes(in: session).first?.id == "child")
}

@MainActor
@Test func aNewAttachmentAcceptsItsFreshTreeRevision() async throws {
    let log = TreeReadLog()
    let model = CharioxAppModel(client: TreeReadClient(tree: try treeFixture(revision: 2), log: log))
    let parent = AgentInstance.fixture(id: "parent", agentRef: "1", alias: "Parent", state: "Idle")
    let session = RuntimeSession.fixture(id: "room", lastUsedAtMs: 2, agents: [parent])
    model.selectSession(session)
    model.agentTrees[parent.id] = try treeFixture(revision: 99)
    model.agentTreeAttachments[parent.id] = "previous-attachment"
    model.activeAttachment = RuntimeAttachment(id: "new-attachment", sessionID: session.id)
    await model.refreshAgentTree(sessionID: session.id, rootAgentID: parent.id)
    #expect(model.agentTrees[parent.id]?.rev == 2)
    #expect(model.agentTreeAttachments[parent.id] == "new-attachment")
}

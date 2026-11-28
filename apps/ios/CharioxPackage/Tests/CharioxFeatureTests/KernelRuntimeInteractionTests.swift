import Foundation
import Testing
@testable import CharioxFeature

private func interactionData(_ subject: [String: Any]) throws -> Data {
    var value: [String: Any] = [
        "id": "decision", "surface": "card", "card_type": "provider_native_permission", "kind": "permission", "level": "warning",
        "message": "Review App installation", "choices": [], "requested_at_ms": 1,
    ]
    value.merge(subject) { _, replacement in replacement }
    return try JSONSerialization.data(withJSONObject: value)
}

@Test func runtimeInteractionDecodesLegacyAgentAndKernelOperationSubjects() throws {
    let decoder = JSONDecoder()
    let agent = try decoder.decode(RuntimeInteraction.self, from: interactionData(["agent_id": "agent"]))
    #expect(agent.agentID == "agent")
    #expect(agent.kernelOperationID == nil)
    let operation = try decoder.decode(RuntimeInteraction.self,
        from: interactionData(["kernel_operation_id": "installation"]))
    #expect(operation.agentID == nil)
    #expect(operation.kernelOperationID == "installation")
    #expect(operation.defaultOnTimeout == nil)
}

@Test func runtimeInteractionRejectsAmbiguousAndInvalidSubjects() throws {
    let invalid: [[String: Any]] = [
        [:], ["agent_id": "agent", "kernel_operation_id": "operation"],
        ["agent_id": "agent", "kernel_operation_id": NSNull()],
        ["kernel_operation_id": NSNull()], ["agent_id": NSNull()],
        ["kernel_operation_id": ""], ["kernel_operation_id": "   "],
        ["kernel_operation_id": "bad\nsubject"], ["kernel_operation_id": 42],
        ["kernel_operation_id": String(repeating: "é", count: 65)],
    ]
    for subject in invalid {
        let data = try interactionData(subject)
        #expect(throws: DecodingError.self) {
            try JSONDecoder().decode(RuntimeInteraction.self, from: data)
        }
    }
}

@Test func questionDecodesBothAnswerShapesAndRecoveryKeepsItsSubject() throws {
    let shapes: [[[String: Any]]] = [[], [["id": "a", "label": "A", "reply": "A"]]]
    for choices in shapes {
        let question = try JSONDecoder().decode(RuntimeInteraction.self, from: interactionData([
            "agent_id": "agent", "kind": "choice", "card_type": "Question", "choices": choices,
            "custom_choice": ["id": "reply", "label": "Something else", "placeholder": "Your route", "min_length": 1],
        ]))
        #expect(question.customChoice?.id == "reply")
        #expect(question.customChoice?.minLength == 1)
        #expect(question.choices.count == choices.count)
    }
    let status = try JSONDecoder().decode(RuntimeInteraction.self, from: interactionData([
        "kernel_operation_id": "deadline", "surface": "pane_status", "related_agent_id": "child", "card_type": NSNull(),
        "choices": [["id": "extend", "label": "Extend", "reply": "extend"]],
    ]))
    #expect(status.surface == "pane_status")
    #expect(status.relatedAgentID == "child")
    #expect(status.choices.first?.id == "extend")
}

@Test func attachedRoomPaneRoutesRecoveryToTheChildAndQuestionToItsHolder() throws {
    let status = try JSONDecoder().decode(RuntimeInteraction.self, from: interactionData([
        "agent_id": "child", "shown_in_agent_id": "parent", "surface": "pane_status", "card_type": NSNull(),
        "choices": [["id": "resume", "label": "Resume", "reply": "resume"], ["id": "fresh", "label": "Start fresh", "reply": "fresh"]],
    ]))
    #expect(status.appears(inAgentPane: "child", defaultAgentID: "parent"))
    #expect(!status.appears(inAgentPane: "parent", defaultAgentID: "parent"))
    let question = try JSONDecoder().decode(RuntimeInteraction.self, from: interactionData([
        "agent_id": "child", "shown_in_agent_id": "parent", "card_type": "Question", "kind": "choice",
        "custom_choice": ["id": "reply", "label": "Something else"],
    ]))
    #expect(question.appears(inAgentPane: "parent", defaultAgentID: "parent"))
    #expect(!question.appears(inAgentPane: "child", defaultAgentID: "parent"))
    #expect(question.customChoice?.id == "reply")
}

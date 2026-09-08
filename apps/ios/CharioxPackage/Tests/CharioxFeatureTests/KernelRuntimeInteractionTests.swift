import Foundation
import Testing
@testable import CharioxFeature

private func interactionData(_ subject: [String: Any]) throws -> Data {
    var value: [String: Any] = [
        "id": "decision", "kind": "permission", "level": "warning",
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

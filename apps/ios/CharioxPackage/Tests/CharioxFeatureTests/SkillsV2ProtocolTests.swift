// MP-08 / MP-10 / MP-11: shared Skills v2 wire snapshots (native compiler required).
import Foundation
import Testing
@testable import CharioxFeature

@Test func skillsV2CatalogUsesSharedKernelEnvelope() throws {
    #expect(skillsMinimumLocalProtocol == 533)
    #expect(skillsMinimumPeerProtocol == 141)
    let request = SkillsV2Request.catalog(.init(project_id: "project-1", agent_id: "agent-1"))
    let data = try KernelProtocolCodec.encodeRequestFrame(
        KernelRequestFrame(requestID: "skills-v2", request: .skillsV2(request))
    )
    let frame = try #require(JSONSerialization.jsonObject(with: data) as? [String: Any])
    let envelope = try #require(frame["request"] as? [String: Any])
    let payload = try #require(envelope["SkillsV2"] as? [String: Any])
    #expect(payload["action"] as? String == "catalog")
    #expect(payload["project_id"] as? String == "project-1")
    #expect(payload["agent_id"] as? String == "agent-1")
}

@Test func skillsV2NativeSelectionRoundTripsBoundIdentity() throws {
    let data = Data(#"{"source":"native","entry_id":"n1","name":"review","provider":"codex","account_profile":"a1","execution_id":"lease1","inventory_revision":"r1"}"#.utf8)
    let selection = try JSONDecoder().decode(SkillSelection.self, from: data)
    guard case let .native(payload) = selection else { Issue.record("Expected provider-bound selection"); return }
    #expect(payload.account_profile == "a1")
    #expect(payload.execution_id == "lease1")
    #expect(payload.inventory_revision == "r1")
    let wire = try #require(JSONSerialization.jsonObject(with: JSONEncoder().encode(selection)) as? [String: Any])
    #expect(wire["path"] == nil)
    #expect(wire["source"] as? String == "native")
}

@Test func skillsV2InheritedStateAndConflictDecode() throws {
    let state = try JSONDecoder().decode(SkillCatalogAction.self, from: Data(#"{"action":"set_state","skill_id":"s1","state":null}"#.utf8))
    guard case let .setState(payload) = state else { Issue.record("Expected state action"); return }
    #expect(payload.state == nil)
    let conflict = try JSONDecoder().decode(SkillDelivery.self, from: Data(#"{"kind":"conflict","provider":"codex","entry_id":"n1","native_digest":"different"}"#.utf8))
    guard case let .conflict(conflictPayload) = conflict else { Issue.record("Expected content conflict"); return }
    #expect(conflictPayload.native_digest == "different")
}

// MP-08 / MP-10 / MP-11: unreleased 533/141 preview receipt and native details.
@Test func skillsV2MutationRequiresReviewReceiptAndNativeDetailsRoundTrip() throws {
    let old = Data(#"{"action":"mutate","mutation":{"scope":{"kind":"chariox"},"expected_revision":0,"actions":[]}}"#.utf8)
    var rejected = false
    do { _ = try JSONDecoder().decode(SkillsV2Request.self, from: old) } catch { rejected = true }
    #expect(rejected)
    let reviewed = Data(#"{"action":"mutate","review_receipt_id":"review-1","mutation":{"scope":{"kind":"chariox"},"expected_revision":0,"actions":[]}}"#.utf8)
    let request = try JSONDecoder().decode(SkillsV2Request.self, from: reviewed)
    let wire = try JSONSerialization.jsonObject(with: JSONEncoder().encode(request)) as! [String: Any]
    #expect(wire["review_receipt_id"] as? String == "review-1")
    let native = Data(#"{"action":"native_details","agent_id":"a","selection":{"source":"native","entry_id":"n","name":"review","provider":"codex","account_profile":"p","execution_id":"e","inventory_revision":"r"}}"#.utf8)
    let details = try JSONDecoder().decode(SkillsV2Request.self, from: native)
    let detailsWire = try JSONSerialization.jsonObject(with: JSONEncoder().encode(details)) as! [String: Any]
    #expect(detailsWire["action"] as? String == "native_details")
    #expect(detailsWire["agent_id"] as? String == "a")
}

@Test func skillsV2CreatesAProjectWithoutInventingAWorkspace() throws {
    let encoded = try JSONEncoder().encode(SkillsV2CreateProjectRequest(name: "No folders"))
    let wire = try JSONSerialization.jsonObject(with: encoded) as! [String: Any]
    let payload = wire["CreateProject"] as! [String: Any]
    #expect(payload["name"] as? String == "No folders")
    #expect(payload["workspace_id"] == nil)
    let response = Data(#"{"ProjectCreated":{"project":{"id":"project-empty","workspace_id":"","workspace_ids":[]}}}"#.utf8)
    _ = try JSONDecoder().decode(SkillsV2ProjectCreatedResponse.self, from: response)
}

// MP-08 / MP-10 / MP-11: foundation versions cannot route users into unavailable RPCs.
@Test func skillsV2RuntimeRequiresWiredKernelAndProtocol() {
    #expect(!supportsSkillsV2Runtime(protocolVersion: 533, capabilities: []))
    #expect(!supportsSkillsV2Runtime(protocolVersion: 532, capabilities: ["skills_v2_runtime"]))
    #expect(supportsSkillsV2Runtime(protocolVersion: 533, capabilities: ["skills_v2_runtime"]))
}

// MP-08 / MP-10 / MP-11: a kernel without the 581 capability must not be offered repository updates.
@Test func skillsRepositoryUpdateRequiresTheRepositoryUpdateCapability() {
    #expect(!supportsSkillsRepositoryUpdate(capabilities: ["skills_v2_runtime"]))
    #expect(!supportsSkillsRepositoryUpdate(capabilities: ["skills_repository_update_v581"]))
    #expect(supportsSkillsRepositoryUpdate(capabilities: ["skills_v2_runtime", "skills_repository_update_v581"]))
}

// MP-08 / MP-10 / MP-11: standalone Project creation, local protocol 533.
import Foundation

public struct SkillsV2CreateProjectRequest: Codable, Equatable, Sendable {
    public struct Payload: Codable, Equatable, Sendable {
        public var name: String
    }
    public var payload: Payload
    private enum CodingKeys: String, CodingKey { case payload = "CreateProject" }
    public init(name: String) { self.payload = Payload(name: name) }
}

public struct SkillsV2ProjectCreatedResponse: Codable, Equatable, Sendable {
    public struct Payload: Codable, Equatable, Sendable {
        // Preserve the shared Project projection, including a genuine [] of
        // Workspaces; a folderless Project never invents a working directory.
        public var project: SkillsV2JSON
    }
    public var payload: Payload
    private enum CodingKeys: String, CodingKey { case payload = "ProjectCreated" }
}

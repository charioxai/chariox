import Foundation

public struct RuntimeInteraction: Identifiable, Equatable, Sendable, Decodable {
    public let id: String
    public let agentID: String?
    public let kernelOperationID: String?
    public let kind: String
    public let level: String
    public let title: String?
    public let message: String
    public let choices: [RuntimeInteractionChoice]
    public let timeoutSeconds: Int?
    public let defaultOnTimeout: String?
    public let requestedAtMs: Int64

    enum CodingKeys: String, CodingKey {
        case id
        case agentID = "agent_id"
        case kernelOperationID = "kernel_operation_id"
        case kind
        case level
        case title
        case message
        case choices
        case timeoutSeconds = "timeout_sec"
        case defaultOnTimeout = "default_on_timeout"
        case requestedAtMs = "requested_at_ms"
    }
}

extension RuntimeInteraction {
    public init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        let hasAgent = values.contains(.agentID)
        let hasOperation = values.contains(.kernelOperationID)
        guard hasAgent != hasOperation else {
            throw DecodingError.dataCorrupted(.init(codingPath: decoder.codingPath,
                debugDescription: "Interaction requires exactly one subject"))
        }
        agentID = hasAgent ? try values.decode(String.self, forKey: .agentID) : nil
        kernelOperationID = hasOperation ? try values.decode(String.self, forKey: .kernelOperationID) : nil
        let subject = agentID ?? kernelOperationID ?? ""
        guard !subject.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
              subject.utf8.count <= 128,
              !subject.unicodeScalars.contains(where: { CharacterSet.controlCharacters.contains($0) }) else {
            throw DecodingError.dataCorrupted(.init(codingPath: decoder.codingPath,
                debugDescription: "Invalid interaction subject"))
        }
        id = try values.decode(String.self, forKey: .id)
        kind = try values.decode(String.self, forKey: .kind)
        level = try values.decode(String.self, forKey: .level)
        title = try values.decodeIfPresent(String.self, forKey: .title)
        message = try values.decode(String.self, forKey: .message)
        choices = try values.decode([RuntimeInteractionChoice].self, forKey: .choices)
        timeoutSeconds = try values.decodeIfPresent(Int.self, forKey: .timeoutSeconds)
        defaultOnTimeout = try values.decodeIfPresent(String.self, forKey: .defaultOnTimeout)
        requestedAtMs = try values.decode(Int64.self, forKey: .requestedAtMs)
    }
}

public struct RuntimeInteractionChoice: Identifiable, Equatable, Sendable, Decodable {
    public let id: String
    public let label: String
    public let reply: String
    public let style: String?
}

import Foundation

public struct RuntimeAgentTree: Equatable, Sendable, Decodable {
    public let root: String
    public let rev: UInt64
    public let nodes: [RuntimeAgentTreeNode]
}

public struct RuntimeAgentTreeNode: Identifiable, Equatable, Sendable, Decodable {
    public let id: String
    public let parentID: String?
    public let roomID: String?
    public let alias: String?
    public let depth: UInt32
    public let state: State
    public let outcome: String?

    public enum State: String, Equatable, Sendable, Decodable {
        case starting, running, waiting, idle, ended, error, unreachable
        case timeLimitEnded = "time_limit_ended"
    }
    enum CodingKeys: String, CodingKey {
        case id = "agent_id", parentID = "parent_agent_id", roomID = "room_id"
        case alias, depth, state, outcome
    }
    public var displayName: String { alias ?? id }
    public var statusText: String {
        switch state {
        case .starting: "Starting"
        case .running: "Running"
        case .waiting: "Waiting"
        case .idle: outcome == "completed" ? "Done" : "Idle"
        case .ended: "Done"
        case .error: "Error"
        case .unreachable: "Unreachable"
        case .timeLimitEnded: "Time limit ended"
        }
    }
}

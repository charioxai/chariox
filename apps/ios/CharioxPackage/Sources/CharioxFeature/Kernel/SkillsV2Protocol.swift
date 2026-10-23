// MP-08 / MP-10 / MP-11: Skills v2 shared wire contract (533/141).
import Foundation
public let skillsMinimumLocalProtocol: UInt32 = 533
public let skillsMinimumPeerProtocol: UInt32 = 141

public enum SkillScope: Codable, Equatable, Sendable {
    case chariox
    case project(ProjectPayload)
    public struct ProjectPayload: Codable, Equatable, Sendable {
        public var project_id: String
    }
    private enum Tag: String, CodingKey { case kind }
    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: Tag.self)
        switch try container.decode(String.self, forKey: .kind) {
        case "chariox": self = .chariox
        case "project": self = .project(try ProjectPayload(from: decoder))
        default: throw DecodingError.dataCorruptedError(forKey: .kind, in: container, debugDescription: "Unknown Skills v2 variant")
        }
    }
    public func encode(to encoder: Encoder) throws {
        var container = encoder.container(keyedBy: Tag.self)
        switch self {
        case .chariox:
            try container.encode("chariox", forKey: .kind)
        case let .project(payload):
            try container.encode("project", forKey: .kind)
            try payload.encode(to: encoder)
        }
    }
}

public enum SkillState: String, Codable, Equatable, Sendable {
    case auto = "auto"
    case manual = "manual"
    case off = "off"
}

public struct SkillFileManifest: Codable, Equatable, Sendable {
    public var path: String
    public var sha256: String
    public var bytes: UInt64
    public var executable: Bool
}

public struct SkillProvenance: Codable, Equatable, Sendable {
    public var kind: String
    public var source_machine: String?
    public var locator: String?
    public var source_revision: String?
    public var provider: String?
    public var account_profile: String?
    public var installed_at_ms: UInt64
    public var updated_at_ms: UInt64
}

public struct SkillRecord: Codable, Equatable, Sendable {
    public var id: String
    public var name: String
    public var scope: SkillScope
    public var package_id: String?
    public var description: String
    public var short_description: String?
    public var default_state: SkillState
    public var state_override: SkillState?
    public var digest: String
    public var declared_version: String?
    public var files: [SkillFileManifest]
    public var provenance: SkillProvenance
    public var compatibility_notes: [String]
}

public struct SkillPackageRecord: Codable, Equatable, Sendable {
    public var id: String
    public var name: String
    public var scope: SkillScope
    public var custom: Bool
    public var source_locator: String?
    public var source_revision: String?
    public var upstream_members: [String: String]
}

public struct SkillCatalog: Codable, Equatable, Sendable {
    public var schema: UInt32
    public var revision: UInt64
    public var scope: SkillScope
    public var skills: [SkillRecord]
    public var packages: [SkillPackageRecord]
    public var chariox_overrides: [String: SkillState]
}

public enum SkillSelection: Codable, Equatable, Sendable {
    case chariox(CharioxPayload)
    public struct CharioxPayload: Codable, Equatable, Sendable {
        public var skill_id: String
        public var name: String
        public var catalog_revision: UInt64
    }
    case native(NativePayload)
    public struct NativePayload: Codable, Equatable, Sendable {
        public var entry_id: String
        public var name: String
        public var provider: String
        public var account_profile: String
        public var execution_id: String
        public var inventory_revision: String
    }
    private enum Tag: String, CodingKey { case source }
    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: Tag.self)
        switch try container.decode(String.self, forKey: .source) {
        case "chariox": self = .chariox(try CharioxPayload(from: decoder))
        case "native": self = .native(try NativePayload(from: decoder))
        default: throw DecodingError.dataCorruptedError(forKey: .source, in: container, debugDescription: "Unknown Skills v2 variant")
        }
    }
    public func encode(to encoder: Encoder) throws {
        var container = encoder.container(keyedBy: Tag.self)
        switch self {
        case let .chariox(payload):
            try container.encode("chariox", forKey: .source)
            try payload.encode(to: encoder)
        case let .native(payload):
            try container.encode("native", forKey: .source)
            try payload.encode(to: encoder)
        }
    }
}

public enum NativeSkillExposure: String, Codable, Equatable, Sendable {
    case confirmed = "confirmed"
    case unverified = "unverified"
    case disabled = "disabled"
}

public struct NativeSkillEntry: Codable, Equatable, Sendable {
    public var selection: SkillSelection
    public var description: String
    public var digest: String?
    public var exposure: NativeSkillExposure
    public var default_state: SkillState
    public var complete_manifest: Bool
    public var supports_load_ack: Bool
    public var discovery_error: String?
}

public struct NativeSkillProvenance: Codable, Equatable, Sendable {
    public var source_machine: String?
    public var locator: String?
    public var provider_version: String?
}

public struct NativeSkillDetails: Codable, Equatable, Sendable {
    public var entry: NativeSkillEntry
    public var provenance: NativeSkillProvenance
    public var body: String?
    public var files: [SkillFileManifest]
    public var declared_version: String?
    public var compatibility_notes: [String]
    public var unavailable_reason: String?
}

public enum SkillDelivery: Codable, Equatable, Sendable {
    case chariox
    case providedBy(ProvidedByPayload)
    public struct ProvidedByPayload: Codable, Equatable, Sendable {
        public var provider: String
        public var entry_id: String
    }
    case conflict(ConflictPayload)
    public struct ConflictPayload: Codable, Equatable, Sendable {
        public var provider: String
        public var entry_id: String
        public var native_digest: String
    }
    case unverified(UnverifiedPayload)
    public struct UnverifiedPayload: Codable, Equatable, Sendable {
        public var provider: String
        public var entry_id: String
    }
    private enum Tag: String, CodingKey { case kind }
    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: Tag.self)
        switch try container.decode(String.self, forKey: .kind) {
        case "chariox": self = .chariox
        case "provided_by": self = .providedBy(try ProvidedByPayload(from: decoder))
        case "conflict": self = .conflict(try ConflictPayload(from: decoder))
        case "unverified": self = .unverified(try UnverifiedPayload(from: decoder))
        default: throw DecodingError.dataCorruptedError(forKey: .kind, in: container, debugDescription: "Unknown Skills v2 variant")
        }
    }
    public func encode(to encoder: Encoder) throws {
        var container = encoder.container(keyedBy: Tag.self)
        switch self {
        case .chariox:
            try container.encode("chariox", forKey: .kind)
        case let .providedBy(payload):
            try container.encode("provided_by", forKey: .kind)
            try payload.encode(to: encoder)
        case let .conflict(payload):
            try container.encode("conflict", forKey: .kind)
            try payload.encode(to: encoder)
        case let .unverified(payload):
            try container.encode("unverified", forKey: .kind)
            try payload.encode(to: encoder)
        }
    }
}

public struct SkillProjection: Codable, Equatable, Sendable {
    public var skill: SkillRecord
    public var effective_state: SkillState
    public var project_override: SkillState?
    public var delivery: SkillDelivery
    public var also_in_provider: String?
}

public enum SkillLoadReason: String, Codable, Equatable, Sendable {
    case explicit = "explicit"
    case model = "model"
    case confirmedNative = "confirmed_native"
}

public struct SkillLoadRecord: Codable, Equatable, Sendable {
    public var skill_id: String
    public var digest: String
    public var session_id: String
    public var agent_id: String
    public var run_id: String
    public var prompt_id: String
    public var reason: SkillLoadReason
    public var context_generation: String
    public var delivered_at_ms: UInt64
}

public enum SkillInstallSource: Codable, Equatable, Sendable {
    case folder(FolderPayload)
    public struct FolderPayload: Codable, Equatable, Sendable {
        public var path: String
    }
    case content(ContentPayload)
    public struct ContentPayload: Codable, Equatable, Sendable {
        public var skill_md: String
    }
    case url(UrlPayload)
    public struct UrlPayload: Codable, Equatable, Sendable {
        public var url: String
        public var pinned_revision: String?
    }
    case native(NativePayload)
    public struct NativePayload: Codable, Equatable, Sendable {
        public var selection: SkillSelection
    }
    case repository(RepositoryPayload)
    public struct RepositoryPayload: Codable, Equatable, Sendable {
        public var url: String
        public var revision: String
        public var path: String
    }
    private enum Tag: String, CodingKey { case kind }
    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: Tag.self)
        switch try container.decode(String.self, forKey: .kind) {
        case "folder": self = .folder(try FolderPayload(from: decoder))
        case "content": self = .content(try ContentPayload(from: decoder))
        case "url": self = .url(try UrlPayload(from: decoder))
        case "native": self = .native(try NativePayload(from: decoder))
        case "repository": self = .repository(try RepositoryPayload(from: decoder))
        default: throw DecodingError.dataCorruptedError(forKey: .kind, in: container, debugDescription: "Unknown Skills v2 variant")
        }
    }
    public func encode(to encoder: Encoder) throws {
        var container = encoder.container(keyedBy: Tag.self)
        switch self {
        case let .folder(payload):
            try container.encode("folder", forKey: .kind)
            try payload.encode(to: encoder)
        case let .content(payload):
            try container.encode("content", forKey: .kind)
            try payload.encode(to: encoder)
        case let .url(payload):
            try container.encode("url", forKey: .kind)
            try payload.encode(to: encoder)
        case let .native(payload):
            try container.encode("native", forKey: .kind)
            try payload.encode(to: encoder)
        case let .repository(payload):
            try container.encode("repository", forKey: .kind)
            try payload.encode(to: encoder)
        }
    }
}

public struct RepositorySkill: Codable, Equatable, Sendable {
    public var path: String
    public var name: String
    public var description: String
}

public struct RepositorySkillSkip: Codable, Equatable, Sendable {
    public var path: String
    public var reason: String
}

public enum SkillCollisionAction: Codable, Equatable, Sendable {
    case reject
    case rename(RenamePayload)
    public struct RenamePayload: Codable, Equatable, Sendable {
        public var name: String
    }
    case replace(ReplacePayload)
    public struct ReplacePayload: Codable, Equatable, Sendable {
        public var skill_id: String
    }
    private enum Tag: String, CodingKey { case action }
    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: Tag.self)
        switch try container.decode(String.self, forKey: .action) {
        case "reject": self = .reject
        case "rename": self = .rename(try RenamePayload(from: decoder))
        case "replace": self = .replace(try ReplacePayload(from: decoder))
        default: throw DecodingError.dataCorruptedError(forKey: .action, in: container, debugDescription: "Unknown Skills v2 variant")
        }
    }
    public func encode(to encoder: Encoder) throws {
        var container = encoder.container(keyedBy: Tag.self)
        switch self {
        case .reject:
            try container.encode("reject", forKey: .action)
        case let .rename(payload):
            try container.encode("rename", forKey: .action)
            try payload.encode(to: encoder)
        case let .replace(payload):
            try container.encode("replace", forKey: .action)
            try payload.encode(to: encoder)
        }
    }
}

public enum SkillCatalogAction: Codable, Equatable, Sendable {
    case install(InstallPayload)
    public struct InstallPayload: Codable, Equatable, Sendable {
        public var source: SkillInstallSource
        public var collision: SkillCollisionAction
        public var package_id: String?
    }
    case update(UpdatePayload)
    public struct UpdatePayload: Codable, Equatable, Sendable {
        public var skill_id: String
        public var source: SkillInstallSource
        public var expected_digest: String
    }
    case remove(RemovePayload)
    public struct RemovePayload: Codable, Equatable, Sendable {
        public var skill_id: String
    }
    case setState(SetStatePayload)
    public struct SetStatePayload: Codable, Equatable, Sendable {
        public var skill_id: String
        public var state: SkillState?
    }
    case createPackage(CreatePackagePayload)
    public struct CreatePackagePayload: Codable, Equatable, Sendable {
        public var package_id: String
        public var name: String
    }
    case renamePackage(RenamePackagePayload)
    public struct RenamePackagePayload: Codable, Equatable, Sendable {
        public var package_id: String
        public var name: String
    }
    case regroup(RegroupPayload)
    public struct RegroupPayload: Codable, Equatable, Sendable {
        public var skill_id: String
        public var package_id: String?
    }
    case removePackage(RemovePackagePayload)
    public struct RemovePackagePayload: Codable, Equatable, Sendable {
        public var package_id: String
        public var remove_skills: Bool
    }
    case setPackageState(SetPackageStatePayload)
    public struct SetPackageStatePayload: Codable, Equatable, Sendable {
        public var package_id: String
        public var state: SkillState?
    }
    case move(MovePayload)
    public struct MovePayload: Codable, Equatable, Sendable {
        public var skill_id: String
        public var destination: SkillScope
    }
    private enum Tag: String, CodingKey { case action }
    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: Tag.self)
        switch try container.decode(String.self, forKey: .action) {
        case "install": self = .install(try InstallPayload(from: decoder))
        case "update": self = .update(try UpdatePayload(from: decoder))
        case "remove": self = .remove(try RemovePayload(from: decoder))
        case "set_state": self = .setState(try SetStatePayload(from: decoder))
        case "create_package": self = .createPackage(try CreatePackagePayload(from: decoder))
        case "rename_package": self = .renamePackage(try RenamePackagePayload(from: decoder))
        case "regroup": self = .regroup(try RegroupPayload(from: decoder))
        case "remove_package": self = .removePackage(try RemovePackagePayload(from: decoder))
        case "set_package_state": self = .setPackageState(try SetPackageStatePayload(from: decoder))
        case "move": self = .move(try MovePayload(from: decoder))
        default: throw DecodingError.dataCorruptedError(forKey: .action, in: container, debugDescription: "Unknown Skills v2 variant")
        }
    }
    public func encode(to encoder: Encoder) throws {
        var container = encoder.container(keyedBy: Tag.self)
        switch self {
        case let .install(payload):
            try container.encode("install", forKey: .action)
            try payload.encode(to: encoder)
        case let .update(payload):
            try container.encode("update", forKey: .action)
            try payload.encode(to: encoder)
        case let .remove(payload):
            try container.encode("remove", forKey: .action)
            try payload.encode(to: encoder)
        case let .setState(payload):
            try container.encode("set_state", forKey: .action)
            try payload.encode(to: encoder)
        case let .createPackage(payload):
            try container.encode("create_package", forKey: .action)
            try payload.encode(to: encoder)
        case let .renamePackage(payload):
            try container.encode("rename_package", forKey: .action)
            try payload.encode(to: encoder)
        case let .regroup(payload):
            try container.encode("regroup", forKey: .action)
            try payload.encode(to: encoder)
        case let .removePackage(payload):
            try container.encode("remove_package", forKey: .action)
            try payload.encode(to: encoder)
        case let .setPackageState(payload):
            try container.encode("set_package_state", forKey: .action)
            try payload.encode(to: encoder)
        case let .move(payload):
            try container.encode("move", forKey: .action)
            try payload.encode(to: encoder)
        }
    }
}

public struct SkillCatalogMutation: Codable, Equatable, Sendable {
    public var scope: SkillScope
    public var expected_revision: UInt64
    public var actions: [SkillCatalogAction]
}

public struct SkillReviewReceipt: Codable, Equatable, Sendable {
    public var id: String
    public var expires_at_ms: UInt64
    public var mutation_sha256: String
    public var sources: [SkillReviewedSource]
}

public struct SkillReviewedSource: Codable, Equatable, Sendable {
    public var action_index: UInt32
    public var skill_id: String
    public var digest: String
    public var source_revision: String?
}

public enum SkillsV2Request: Codable, Equatable, Sendable {
    case catalog(CatalogPayload)
    public struct CatalogPayload: Codable, Equatable, Sendable {
        public var project_id: String?
        public var agent_id: String?
    }
    case get(GetPayload)
    public struct GetPayload: Codable, Equatable, Sendable {
        public var skill_id: String
        public var project_id: String?
    }
    case nativeInventory(NativeInventoryPayload)
    public struct NativeInventoryPayload: Codable, Equatable, Sendable {
        public var agent_id: String
        public var refresh: Bool
    }
    case nativeDetails(NativeDetailsPayload)
    public struct NativeDetailsPayload: Codable, Equatable, Sendable {
        public var agent_id: String
        public var selection: SkillSelection
    }
    case loads(LoadsPayload)
    public struct LoadsPayload: Codable, Equatable, Sendable {
        public var session_id: String
        public var agent_id: String?
    }
    case mutate(MutatePayload)
    public struct MutatePayload: Codable, Equatable, Sendable {
        public var mutation: SkillCatalogMutation
        public var review_receipt_id: String
    }
    case preview(PreviewPayload)
    public struct PreviewPayload: Codable, Equatable, Sendable {
        public var mutation: SkillCatalogMutation
    }
    case resolveRepository(ResolveRepositoryPayload)
    public struct ResolveRepositoryPayload: Codable, Equatable, Sendable {
        public var url: String
    }
    case resolveRepositoryUpdate(ResolveRepositoryUpdatePayload)
    public struct ResolveRepositoryUpdatePayload: Codable, Equatable, Sendable {
        public var package_id: String
    }
    case list(ListPayload)
    public struct ListPayload: Codable, Equatable, Sendable {
        public var session_id: String
        public var agent_id: String
        public var cursor: String?
        public var limit: UInt32
    }
    case load(LoadPayload)
    public struct LoadPayload: Codable, Equatable, Sendable {
        public var session_id: String
        public var agent_id: String
        public var skill_id: String
    }
    case submit(SubmitPayload)
    public struct SubmitPayload: Codable, Equatable, Sendable {
        public var session_id: String
        public var attachment_id: String
        public var target_agent_id: String
        public var prompt: String
        public var attachments: [SkillsV2PromptAttachment]
        public var selections: [SkillSelection]
    }
    private enum Tag: String, CodingKey { case action }
    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: Tag.self)
        switch try container.decode(String.self, forKey: .action) {
        case "catalog": self = .catalog(try CatalogPayload(from: decoder))
        case "get": self = .get(try GetPayload(from: decoder))
        case "native_inventory": self = .nativeInventory(try NativeInventoryPayload(from: decoder))
        case "native_details": self = .nativeDetails(try NativeDetailsPayload(from: decoder))
        case "loads": self = .loads(try LoadsPayload(from: decoder))
        case "mutate": self = .mutate(try MutatePayload(from: decoder))
        case "preview": self = .preview(try PreviewPayload(from: decoder))
        case "resolve_repository": self = .resolveRepository(try ResolveRepositoryPayload(from: decoder))
        case "resolve_repository_update": self = .resolveRepositoryUpdate(try ResolveRepositoryUpdatePayload(from: decoder))
        case "list": self = .list(try ListPayload(from: decoder))
        case "load": self = .load(try LoadPayload(from: decoder))
        case "submit": self = .submit(try SubmitPayload(from: decoder))
        default: throw DecodingError.dataCorruptedError(forKey: .action, in: container, debugDescription: "Unknown Skills v2 variant")
        }
    }
    public func encode(to encoder: Encoder) throws {
        var container = encoder.container(keyedBy: Tag.self)
        switch self {
        case let .catalog(payload):
            try container.encode("catalog", forKey: .action)
            try payload.encode(to: encoder)
        case let .get(payload):
            try container.encode("get", forKey: .action)
            try payload.encode(to: encoder)
        case let .nativeInventory(payload):
            try container.encode("native_inventory", forKey: .action)
            try payload.encode(to: encoder)
        case let .nativeDetails(payload):
            try container.encode("native_details", forKey: .action)
            try payload.encode(to: encoder)
        case let .loads(payload):
            try container.encode("loads", forKey: .action)
            try payload.encode(to: encoder)
        case let .mutate(payload):
            try container.encode("mutate", forKey: .action)
            try payload.encode(to: encoder)
        case let .preview(payload):
            try container.encode("preview", forKey: .action)
            try payload.encode(to: encoder)
        case let .resolveRepository(payload):
            try container.encode("resolve_repository", forKey: .action)
            try payload.encode(to: encoder)
        case let .resolveRepositoryUpdate(payload):
            try container.encode("resolve_repository_update", forKey: .action)
            try payload.encode(to: encoder)
        case let .list(payload):
            try container.encode("list", forKey: .action)
            try payload.encode(to: encoder)
        case let .load(payload):
            try container.encode("load", forKey: .action)
            try payload.encode(to: encoder)
        case let .submit(payload):
            try container.encode("submit", forKey: .action)
            try payload.encode(to: encoder)
        }
    }
}

public enum SkillsV2Response: Codable, Equatable, Sendable {
    case catalog(CatalogPayload)
    public struct CatalogPayload: Codable, Equatable, Sendable {
        public var revision: UInt64
        public var skills: [SkillProjection]
        public var packages: [SkillPackageRecord]
        public var native: [NativeSkillEntry]
        public var metadata_budget_percent: Double
    }
    case detail(DetailPayload)
    public struct DetailPayload: Codable, Equatable, Sendable {
        public var skill: SkillProjection
        public var body: String
    }
    case nativeInventory(NativeInventoryPayload)
    public struct NativeInventoryPayload: Codable, Equatable, Sendable {
        public var entries: [NativeSkillEntry]
        public var inventory_revision: String
    }
    case nativeDetails(NativeDetailsPayload)
    public struct NativeDetailsPayload: Codable, Equatable, Sendable {
        public var details: NativeSkillDetails
    }
    case loads(LoadsPayload)
    public struct LoadsPayload: Codable, Equatable, Sendable {
        public var records: [SkillLoadRecord]
    }
    case preview(PreviewPayload)
    public struct PreviewPayload: Codable, Equatable, Sendable {
        public var expected_revision: UInt64
        public var affected: [SkillRecord]
        public var collisions: [String]
        public var review_receipt: SkillReviewReceipt
    }
    case repository(RepositoryPayload)
    public struct RepositoryPayload: Codable, Equatable, Sendable {
        public var locator: String
        public var revision: String
        public var skills: [RepositorySkill]
        public var skipped: [RepositorySkillSkip]
    }
    case repositoryUpdate(RepositoryUpdatePayload)
    public struct RepositoryUpdatePayload: Codable, Equatable, Sendable {
        public var package_id: String
        public var locator: String
        public var from_revision: String
        public var revision: String
        public var changed: [RepositorySkillUpdate]
        public var added: [RepositorySkill]
        public var removed: [RepositorySkillMember]
        public var unchanged: [RepositorySkillMember]
        public var skipped: [RepositorySkillSkip]
        public var mutation: SkillCatalogMutation
    }
    case published(PublishedPayload)
    public struct PublishedPayload: Codable, Equatable, Sendable {
        public var revision: UInt64
        public var changed_skill_ids: [String]
        public var removed_skill_ids: [String]
    }
    case page(PagePayload)
    public struct PagePayload: Codable, Equatable, Sendable {
        public var revision: UInt64
        public var skills: [SkillProjection]
        public var next_cursor: String?
    }
    case loaded(LoadedPayload)
    public struct LoadedPayload: Codable, Equatable, Sendable {
        public var skill_id: String
        public var digest: String
        public var body: String
        public var root: String
        public var files: [SkillFileManifest]
    }
    case submitted(SubmittedPayload)
    public struct SubmittedPayload: Codable, Equatable, Sendable {
        public var outcome: SkillsV2PromptOutcome
    }
    private enum Tag: String, CodingKey { case kind }
    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: Tag.self)
        switch try container.decode(String.self, forKey: .kind) {
        case "catalog": self = .catalog(try CatalogPayload(from: decoder))
        case "detail": self = .detail(try DetailPayload(from: decoder))
        case "native_inventory": self = .nativeInventory(try NativeInventoryPayload(from: decoder))
        case "native_details": self = .nativeDetails(try NativeDetailsPayload(from: decoder))
        case "loads": self = .loads(try LoadsPayload(from: decoder))
        case "preview": self = .preview(try PreviewPayload(from: decoder))
        case "repository": self = .repository(try RepositoryPayload(from: decoder))
        case "repository_update": self = .repositoryUpdate(try RepositoryUpdatePayload(from: decoder))
        case "published": self = .published(try PublishedPayload(from: decoder))
        case "page": self = .page(try PagePayload(from: decoder))
        case "loaded": self = .loaded(try LoadedPayload(from: decoder))
        case "submitted": self = .submitted(try SubmittedPayload(from: decoder))
        default: throw DecodingError.dataCorruptedError(forKey: .kind, in: container, debugDescription: "Unknown Skills v2 variant")
        }
    }
    public func encode(to encoder: Encoder) throws {
        var container = encoder.container(keyedBy: Tag.self)
        switch self {
        case let .catalog(payload):
            try container.encode("catalog", forKey: .kind)
            try payload.encode(to: encoder)
        case let .detail(payload):
            try container.encode("detail", forKey: .kind)
            try payload.encode(to: encoder)
        case let .nativeInventory(payload):
            try container.encode("native_inventory", forKey: .kind)
            try payload.encode(to: encoder)
        case let .nativeDetails(payload):
            try container.encode("native_details", forKey: .kind)
            try payload.encode(to: encoder)
        case let .loads(payload):
            try container.encode("loads", forKey: .kind)
            try payload.encode(to: encoder)
        case let .preview(payload):
            try container.encode("preview", forKey: .kind)
            try payload.encode(to: encoder)
        case let .repository(payload):
            try container.encode("repository", forKey: .kind)
            try payload.encode(to: encoder)
        case let .repositoryUpdate(payload):
            try container.encode("repository_update", forKey: .kind)
            try payload.encode(to: encoder)
        case let .published(payload):
            try container.encode("published", forKey: .kind)
            try payload.encode(to: encoder)
        case let .page(payload):
            try container.encode("page", forKey: .kind)
            try payload.encode(to: encoder)
        case let .loaded(payload):
            try container.encode("loaded", forKey: .kind)
            try payload.encode(to: encoder)
        case let .submitted(payload):
            try container.encode("submitted", forKey: .kind)
            try payload.encode(to: encoder)
        }
    }
}

public enum SkillCatalogEvent: Codable, Equatable, Sendable {
    case changed(ChangedPayload)
    public struct ChangedPayload: Codable, Equatable, Sendable {
        public var revision: UInt64
        public var scope: SkillScope
        public var changed_skill_ids: [String]
        public var removed_skill_ids: [String]
    }
    case loaded(LoadedPayload)
    public struct LoadedPayload: Codable, Equatable, Sendable {
        public var record: SkillLoadRecord
    }
    private enum Tag: String, CodingKey { case kind }
    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: Tag.self)
        switch try container.decode(String.self, forKey: .kind) {
        case "changed": self = .changed(try ChangedPayload(from: decoder))
        case "loaded": self = .loaded(try LoadedPayload(from: decoder))
        default: throw DecodingError.dataCorruptedError(forKey: .kind, in: container, debugDescription: "Unknown Skills v2 variant")
        }
    }
    public func encode(to encoder: Encoder) throws {
        var container = encoder.container(keyedBy: Tag.self)
        switch self {
        case let .changed(payload):
            try container.encode("changed", forKey: .kind)
            try payload.encode(to: encoder)
        case let .loaded(payload):
            try container.encode("loaded", forKey: .kind)
            try payload.encode(to: encoder)
        }
    }
}

public struct SkillTransferManifest: Codable, Equatable, Sendable {
    public var catalog_revision: UInt64
    public var project_id: String?
    public var skills: [SkillTransferEntry]
}

public struct SkillTransferEntry: Codable, Equatable, Sendable {
    public var skill_id: String
    public var digest: String
    public var scope: SkillScope
    public var state: SkillState
}

public struct SkillEnvironmentSnapshot: Codable, Equatable, Sendable {
    public var schema: UInt32
    public var source_revision: UInt64
    public var project: SkillCatalog
    public var chariox_dependencies: [SkillRecord]
    public var dependency_packages: [SkillPackageRecord]
}

public struct SkillTurnContext: Codable, Equatable, Sendable {
    public var catalog_revision: UInt64
    public var context_generation: String
    public var metadata: String
    public var metadata_tokens_upper_bound: UInt64
    public var metadata_budget_tokens: UInt64
    public var omitted_auto_count: UInt32
    public var loaded: [SkillResolvedBody]
    public var native_selections: [SkillSelection]
}

public struct SkillResolvedBody: Codable, Equatable, Sendable {
    public var skill_id: String
    public var name: String
    public var digest: String
    public var root: String
    public var body: String
    public var files: [SkillFileManifest]
    public var reason: SkillLoadReason
    public var delivery: SkillDelivery
}

public struct SkillsV2PromptAttachment: Codable, Equatable, Sendable {
    public var url: String
    public var mime: String
    public var filename: String?
    public var contents_base64: String?
}
public enum SkillsV2PromptOutcome: Codable, Equatable, Sendable {
    case started(SkillsV2JSON), queued(SkillsV2JSON)
    private enum Keys: String, CodingKey { case Started, Queued }
    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: Keys.self)
        if c.contains(.Started) { self = .started(try c.decode(SkillsV2JSON.self, forKey: .Started)) }
        else { self = .queued(try c.decode(SkillsV2JSON.self, forKey: .Queued)) }
    }
    public func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: Keys.self)
        switch self { case .started(let p): try c.encode(p, forKey: .Started); case .queued(let p): try c.encode(p, forKey: .Queued) }
    }
}
public indirect enum SkillsV2JSON: Codable, Equatable, Sendable {
    case object([String: SkillsV2JSON]), array([SkillsV2JSON]), string(String), number(Double), bool(Bool), null
    public init(from decoder: Decoder) throws {
        let c = try decoder.singleValueContainer()
        if c.decodeNil() { self = .null }
        else if let v = try? c.decode(Bool.self) { self = .bool(v) }
        else if let v = try? c.decode(String.self) { self = .string(v) }
        else if let v = try? c.decode(Double.self) { self = .number(v) }
        else if let v = try? c.decode([String: SkillsV2JSON].self) { self = .object(v) }
        else { self = .array(try c.decode([SkillsV2JSON].self)) }
    }
    public func encode(to encoder: Encoder) throws {
        var c = encoder.singleValueContainer()
        switch self {
        case .object(let v): try c.encode(v)
        case .array(let v): try c.encode(v)
        case .string(let v): try c.encode(v)
        case .number(let v): try c.encode(v)
        case .bool(let v): try c.encode(v)
        case .null: try c.encodeNil()
        }
    }
}

// MP-08 / MP-10 / MP-11: a repository diff is reviewed before the normal atomic batch.
public struct RepositorySkillMember: Codable, Equatable, Sendable {
    public var path: String
    public var skill_id: String
    public var name: String
    public var digest: String
}
public struct RepositorySkillUpdate: Codable, Equatable, Sendable {
    public var member: RepositorySkillMember
    public var digest: String
}

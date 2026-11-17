// MP-08 / MP-10 / MP-11: protocol support does not imply a wired Skills runtime.
public let skillsRuntimeCapability = "skills_v2_runtime"
public func supportsSkillsV2Runtime(protocolVersion: UInt32, capabilities: [String]) -> Bool {
    protocolVersion >= skillsMinimumLocalProtocol && capabilities.contains(skillsRuntimeCapability)
}
// MP-08 / MP-10 / MP-11: local 581 adds resolve_repository_update; a 571 kernel advertises the runtime without it.
public let skillsRepositoryUpdateCapability = "skills_repository_update_v581"
public func supportsSkillsRepositoryUpdate(capabilities: [String]) -> Bool {
    capabilities.contains(skillsRuntimeCapability) && capabilities.contains(skillsRepositoryUpdateCapability)
}

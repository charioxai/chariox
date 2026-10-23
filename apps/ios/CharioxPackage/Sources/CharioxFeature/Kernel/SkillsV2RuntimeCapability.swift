// MP-08 / MP-10 / MP-11: protocol support does not imply a wired Skills runtime.
public let skillsRuntimeCapability = "skills_v2_runtime"
public func supportsSkillsV2Runtime(protocolVersion: UInt32, capabilities: [String]) -> Bool {
    protocolVersion >= skillsMinimumLocalProtocol && capabilities.contains(skillsRuntimeCapability)
}

import Foundation
import Testing
@testable import ArrobaFeature

@Test func listSessionsRequestMatchesKernelEnvelope() throws {
    let data = try KernelProtocolCodec.encodeRequestFrame(
        KernelRequestFrame(requestID: "request-1", request: .listSessions)
    )
    let object = try #require(
        JSONSerialization.jsonObject(with: data) as? [String: Any]
    )

    #expect(object["type"] as? String == "request")
    #expect(object["request_id"] as? String == "request-1")
    let request = try #require(object["request"] as? [String: Any])
    #expect(request.keys.contains("ListSessions"))
    #expect(request["ListSessions"] is NSNull)
}

@Test func createSessionRequestMatchesKernelShape() throws {
    let data = try KernelProtocolCodec.encodeRequestFrame(
        KernelRequestFrame(
            requestID: "request-2",
            request: .createSession(
                workspaceID: "/Users/miguel/arroba",
                worktreeID: "/Users/miguel/arroba",
                alias: "ios"
            )
        )
    )
    let object = try #require(
        JSONSerialization.jsonObject(with: data) as? [String: Any]
    )
    let request = try #require(object["request"] as? [String: Any])
    let payload = try #require(request["CreateSession"] as? [String: Any])

    #expect(payload["workspace_id"] as? String == "/Users/miguel/arroba")
    #expect(payload["worktree_id"] as? String == "/Users/miguel/arroba")
    #expect(payload["alias"] as? String == "ios")
}

@Test func deleteSessionRequestMatchesKernelShape() throws {
    let data = try KernelProtocolCodec.encodeRequestFrame(
        KernelRequestFrame(
            requestID: "request-delete",
            request: .deleteSession(sessionRef: "session-1", workspaceID: "/repo")
        )
    )
    let object = try #require(
        JSONSerialization.jsonObject(with: data) as? [String: Any]
    )
    let request = try #require(object["request"] as? [String: Any])
    let payload = try #require(request["DeleteSession"] as? [String: Any])

    #expect(payload["session_ref"] as? String == "session-1")
    #expect(payload["workspace_id"] as? String == "/repo")
}

@Test func subscribeFrameMatchesKernelTransportShape() throws {
    let data = try KernelProtocolCodec.encodeSubscribeFrame(
        KernelSubscribeFrame(
            requestID: "subscribe-1",
            sessionID: "session-1",
            attachmentID: "attachment-1",
            resumeFromEventID: 42
        )
    )
    let object = try #require(
        JSONSerialization.jsonObject(with: data) as? [String: Any]
    )

    #expect(object["type"] as? String == "subscribe")
    #expect(object["request_id"] as? String == "subscribe-1")
    #expect(object["session_id"] as? String == "session-1")
    #expect(object["attachment_id"] as? String == "attachment-1")
    #expect(object["resume_from_event_id"] as? Int == 42)
}

@Test func detachSessionRequestMatchesKernelShape() throws {
    let data = try KernelProtocolCodec.encodeRequestFrame(
        KernelRequestFrame(
            requestID: "request-detach",
            request: .detachFromSession(attachmentID: "attachment-1")
        )
    )
    let object = try #require(
        JSONSerialization.jsonObject(with: data) as? [String: Any]
    )
    let request = try #require(object["request"] as? [String: Any])
    let payload = try #require(request["DetachFromSession"] as? [String: Any])

    #expect(payload["attachment_id"] as? String == "attachment-1")
}

@Test func updateSessionConfigRequestMatchesKernelShape() throws {
    let data = try KernelProtocolCodec.encodeRequestFrame(
        KernelRequestFrame(
            requestID: "request-session-config",
            request: .updateSessionConfig(
                sessionID: "session-1",
                attachmentID: "attachment-1",
                values: ["agents.mode": "plan"],
                requiresIdle: false
            )
        )
    )
    let object = try #require(
        JSONSerialization.jsonObject(with: data) as? [String: Any]
    )
    let request = try #require(object["request"] as? [String: Any])
    let payload = try #require(request["UpdateSessionConfig"] as? [String: Any])
    let values = try #require(payload["values"] as? [String: String])

    #expect(payload["session_id"] as? String == "session-1")
    #expect(payload["attachment_id"] as? String == "attachment-1")
    #expect(values["agents.mode"] == "plan")
    #expect(payload["requires_idle"] as? Bool == false)
}

@Test func workspaceLiveSyncRequestsMatchKernelShape() throws {
    let statusData = try KernelProtocolCodec.encodeRequestFrame(
        KernelRequestFrame(
            requestID: "request-workspace-sync",
            request: .getWorkspaceLiveSyncStatus(sessionID: "session-1")
        )
    )
    let statusObject = try #require(
        JSONSerialization.jsonObject(with: statusData) as? [String: Any]
    )
    let statusRequest = try #require(statusObject["request"] as? [String: Any])
    let statusPayload = try #require(statusRequest["GetWorkspaceLiveSyncStatus"] as? [String: Any])
    #expect(statusPayload["session_id"] as? String == "session-1")

    let modeData = try KernelProtocolCodec.encodeRequestFrame(
        KernelRequestFrame(
            requestID: "request-workspace-sync-mode",
            request: .setWorkspaceLiveSyncMode(sessionID: "session-1", mode: "tracked")
        )
    )
    let modeObject = try #require(
        JSONSerialization.jsonObject(with: modeData) as? [String: Any]
    )
    let modeRequest = try #require(modeObject["request"] as? [String: Any])
    let modePayload = try #require(modeRequest["SetWorkspaceLiveSyncMode"] as? [String: Any])
    #expect(modePayload["session_id"] as? String == "session-1")
    #expect(modePayload["mode"] as? String == "tracked")

    let linkData = try KernelProtocolCodec.encodeRequestFrame(
        KernelRequestFrame(
            requestID: "request-workspace-sync-link",
            request: .attachWorkspaceLink(sessionID: "session-1", linkRef: "shared", repoRoot: "/repo")
        )
    )
    let linkObject = try #require(
        JSONSerialization.jsonObject(with: linkData) as? [String: Any]
    )
    let linkRequest = try #require(linkObject["request"] as? [String: Any])
    let linkPayload = try #require(linkRequest["AttachWorkspaceLink"] as? [String: Any])
    #expect(linkPayload["session_id"] as? String == "session-1")
    #expect(linkPayload["link_ref"] as? String == "shared")
    #expect(linkPayload["repo_root"] as? String == "/repo")
}

@Test func workspaceLiveSyncResponsesDecode() throws {
    let statusJSON = """
    {
      "type": "response",
      "request_id": "request-workspace-sync",
      "response": {
        "WorkspaceLiveSyncStatus": {
          "status": {
            "session_id": "session-1",
            "mode": "tracked",
            "footer_state": "conflict",
            "sync_groups": [{
              "group_id": "link-1",
              "group_name": "shared",
              "target_count": 1,
              "ready_targets": 0,
              "degraded_targets": 0,
              "conflicted_targets": 1
            }],
            "targets": [{
              "link_id": "link-1",
              "link_name": "shared",
              "user_id": "user-2",
              "machine_id": "machine-2",
              "kernel_id": "kernel-2",
              "repo_root": "/repo",
              "branch": "main",
              "repo_fingerprint": null,
              "status": "conflict",
              "attached_at_ms": 42
            }],
            "conflicts": [{
              "conflict_id": "conflict-1",
              "link_id": "link-1",
              "source_agent_id": "agent-1",
              "target_user_id": "user-2",
              "target_repo_root": "/repo",
              "path": "src/app.swift",
              "next_action": "reconcile and retry"
            }],
            "ignore": {
              "ignore_file": "/repo/.arrobaignore",
              "rules": ["ignored/**", "*.secret"],
              "force_excludes": [".git/**", ".arroba/**"]
            }
          }
        }
      },
      "error": null
    }
    """

    let statusFrame = try KernelProtocolCodec.decodeResponseFrame(Data(statusJSON.utf8))
    guard case let .workspaceLiveSyncStatus(status) = statusFrame.response else {
        Issue.record("Expected WorkspaceLiveSyncStatus response")
        return
    }
    #expect(status.mode == "tracked")
    #expect(status.footerState == "conflict")
    #expect(status.syncGroups.first?.groupName == "shared")
    #expect(status.targets.first?.linkName == "shared")
    #expect(status.conflicts.first?.path == "src/app.swift")
    #expect(status.ignore.rules == ["ignored/**", "*.secret"])
    #expect(status.ignore.forceExcludes.contains(".git/**"))

    let updatedJSON = """
    {
      "type": "response",
      "request_id": "request-workspace-sync-mode",
      "response": {
        "WorkspaceLiveSyncModeUpdated": {
          "session": {
            "id": "session-1",
            "alias": null,
            "workspace_id": "/repo",
            "worktree_id": "/repo",
            "status": "Active",
            "focused_agent_id": null,
            "workspace_live_sync_mode": "tracked",
            "created_at_ms": 1777111200000,
            "last_used_at_ms": 1777111201000,
            "agents": []
          }
        }
      },
      "error": null
    }
    """

    let updatedFrame = try KernelProtocolCodec.decodeResponseFrame(Data(updatedJSON.utf8))
    guard case let .workspaceLiveSyncModeUpdated(session) = updatedFrame.response else {
        Issue.record("Expected WorkspaceLiveSyncModeUpdated response")
        return
    }
    #expect(session.workspaceLiveSyncMode == "tracked")
}

@Test func getProviderCatalogRequestMatchesKernelShape() throws {
    let data = try KernelProtocolCodec.encodeRequestFrame(
        KernelRequestFrame(
            requestID: "request-provider-catalog",
            request: .getProviderCatalog
        )
    )
    let object = try #require(
        JSONSerialization.jsonObject(with: data) as? [String: Any]
    )
    let request = try #require(object["request"] as? [String: Any])

    #expect(request.keys.contains("GetProviderCatalog"))
    #expect(request["GetProviderCatalog"] is NSNull)
}

@Test func getProviderAuthStatusRequestMatchesKernelShape() throws {
    let data = try KernelProtocolCodec.encodeRequestFrame(
        KernelRequestFrame(
            requestID: "request-provider-auth",
            request: .getProviderAuthStatus(provider: "codex")
        )
    )
    let object = try #require(
        JSONSerialization.jsonObject(with: data) as? [String: Any]
    )
    let request = try #require(object["request"] as? [String: Any])
    let payload = try #require(request["GetProviderAuthStatus"] as? [String: Any])

    #expect(payload["provider"] as? String == "codex")
}

@Test func providerLoginAndLogoutRequestsMatchKernelShape() throws {
    let loginData = try KernelProtocolCodec.encodeRequestFrame(
        KernelRequestFrame(
            requestID: "request-provider-login",
            request: .startProviderLogin(provider: "codex")
        )
    )
    let loginObject = try #require(
        JSONSerialization.jsonObject(with: loginData) as? [String: Any]
    )
    let loginRequest = try #require(loginObject["request"] as? [String: Any])
    let loginPayload = try #require(loginRequest["StartProviderLogin"] as? [String: Any])
    #expect(loginPayload["provider"] as? String == "codex")

    let logoutData = try KernelProtocolCodec.encodeRequestFrame(
        KernelRequestFrame(
            requestID: "request-provider-logout",
            request: .logoutProvider(provider: "codex")
        )
    )
    let logoutObject = try #require(
        JSONSerialization.jsonObject(with: logoutData) as? [String: Any]
    )
    let logoutRequest = try #require(logoutObject["request"] as? [String: Any])
    let logoutPayload = try #require(logoutRequest["LogoutProvider"] as? [String: Any])
    #expect(logoutPayload["provider"] as? String == "codex")
}

@Test func providerLoginAndLogoutResponsesDecode() throws {
    let loginJSON = """
    {
      "type": "response",
      "request_id": "request-provider-login",
      "response": {
        "ProviderLoginStarted": {
          "login": {
            "provider": "codex",
            "login_kind": "device",
            "login_id": "login-1",
            "auth_url": null,
            "verification_url": "https://example.com/activate",
            "user_code": "ABCD-EFGH"
          }
        }
      },
      "error": null
    }
    """

    let loginFrame = try KernelProtocolCodec.decodeResponseFrame(Data(loginJSON.utf8))
    guard case let .providerLoginStarted(login) = loginFrame.response else {
        Issue.record("Expected ProviderLoginStarted response")
        return
    }
    #expect(login.provider == "codex")
    #expect(login.userCode == "ABCD-EFGH")

    let logoutJSON = """
    {
      "type": "response",
      "request_id": "request-provider-logout",
      "response": {
        "ProviderLoggedOut": {
          "provider": "codex"
        }
      },
      "error": null
    }
    """
    let logoutFrame = try KernelProtocolCodec.decodeResponseFrame(Data(logoutJSON.utf8))
    guard case let .providerLoggedOut(provider) = logoutFrame.response else {
        Issue.record("Expected ProviderLoggedOut response")
        return
    }
    #expect(provider == "codex")
}

@Test func listMcpServersRequestMatchesKernelShape() throws {
    let data = try KernelProtocolCodec.encodeRequestFrame(
        KernelRequestFrame(
            requestID: "request-mcp-list",
            request: .listMcpServers(workspaceID: "/repo")
        )
    )
    let object = try #require(
        JSONSerialization.jsonObject(with: data) as? [String: Any]
    )
    let request = try #require(object["request"] as? [String: Any])
    let payload = try #require(request["ListMcpServers"] as? [String: Any])

    #expect(payload["workspace_id"] as? String == "/repo")
}

@Test func listSkillsRequestMatchesKernelShape() throws {
    let data = try KernelProtocolCodec.encodeRequestFrame(
        KernelRequestFrame(
            requestID: "request-skill-list",
            request: .listSkills(workspaceID: nil)
        )
    )
    let object = try #require(
        JSONSerialization.jsonObject(with: data) as? [String: Any]
    )
    let request = try #require(object["request"] as? [String: Any])
    let payload = try #require(request["ListSkills"] as? [String: Any])

    #expect(payload["workspace_id"] is NSNull)
}

@Test func respondToInteractionRequestMatchesKernelShape() throws {
    let data = try KernelProtocolCodec.encodeRequestFrame(
        KernelRequestFrame(
            requestID: "request-interaction",
            request: .respondToInteraction(
                sessionID: "session-1",
                interactionID: "interaction-1",
                choiceID: "approve"
            )
        )
    )
    let object = try #require(
        JSONSerialization.jsonObject(with: data) as? [String: Any]
    )
    let request = try #require(object["request"] as? [String: Any])
    let payload = try #require(request["RespondToInteraction"] as? [String: Any])

    #expect(payload["session_id"] as? String == "session-1")
    #expect(payload["interaction_id"] as? String == "interaction-1")
    #expect(payload["choice_id"] as? String == "approve")
}

@Test func submitPromptRequestMatchesKernelShape() throws {
    let data = try KernelProtocolCodec.encodeRequestFrame(
        KernelRequestFrame(
            requestID: "request-prompt",
            request: .submitPrompt(
                sessionID: "session-1",
                attachmentID: "attachment-1",
                targetAgentID: "agent-1",
                prompt: "Build the iOS app.\n"
            )
        )
    )
    let object = try #require(
        JSONSerialization.jsonObject(with: data) as? [String: Any]
    )
    let request = try #require(object["request"] as? [String: Any])
    let payload = try #require(request["SubmitPrompt"] as? [String: Any])

    #expect(payload["session_id"] as? String == "session-1")
    #expect(payload["attachment_id"] as? String == "attachment-1")
    #expect(payload["target_agent_id"] as? String == "agent-1")
    #expect(payload["prompt"] as? String == "Build the iOS app.\n")
    #expect((payload["attachments"] as? [Any])?.isEmpty == true)
}

@Test func getSessionHistoryRequestMatchesKernelShape() throws {
    let data = try KernelProtocolCodec.encodeRequestFrame(
        KernelRequestFrame(
            requestID: "request-history",
            request: .getSessionHistory(
                sessionID: "session-1",
                agentID: "agent-1",
                roundCount: 8,
                maxChars: 80_000
            )
        )
    )
    let object = try #require(
        JSONSerialization.jsonObject(with: data) as? [String: Any]
    )
    let request = try #require(object["request"] as? [String: Any])
    let payload = try #require(request["GetSessionHistory"] as? [String: Any])

    #expect(payload["session_id"] as? String == "session-1")
    #expect(payload["agent_id"] as? String == "agent-1")
    #expect(payload["round_count"] as? Int == 8)
    #expect(payload["max_chars"] as? Int == 80_000)
}

@Test func focusAgentRequestMatchesKernelShape() throws {
    let data = try KernelProtocolCodec.encodeRequestFrame(
        KernelRequestFrame(
            requestID: "request-focus",
            request: .focusAgent(sessionID: "session-1", agentID: "agent-2")
        )
    )
    let object = try #require(
        JSONSerialization.jsonObject(with: data) as? [String: Any]
    )
    let request = try #require(object["request"] as? [String: Any])
    let payload = try #require(request["FocusAgent"] as? [String: Any])

    #expect(payload["session_id"] as? String == "session-1")
    #expect(payload["agent_id"] as? String == "agent-2")
}

@Test func cycleAgentFocusRequestMatchesKernelShape() throws {
    let data = try KernelProtocolCodec.encodeRequestFrame(
        KernelRequestFrame(
            requestID: "request-cycle",
            request: .cycleAgentFocus(sessionID: "session-1")
        )
    )
    let object = try #require(
        JSONSerialization.jsonObject(with: data) as? [String: Any]
    )
    let request = try #require(object["request"] as? [String: Any])
    let payload = try #require(request["CycleAgentFocus"] as? [String: Any])

    #expect(payload["session_id"] as? String == "session-1")
}

@Test func spawnAgentRequestMatchesKernelShape() throws {
    let data = try KernelProtocolCodec.encodeRequestFrame(
        KernelRequestFrame(
            requestID: "request-spawn",
            request: .spawnAgent(
                sessionID: "session-1",
                alias: "reviewer",
                provider: "opencode",
                model: "default",
                effort: "high",
                worktreeID: nil
            )
        )
    )
    let object = try #require(
        JSONSerialization.jsonObject(with: data) as? [String: Any]
    )
    let request = try #require(object["request"] as? [String: Any])
    let payload = try #require(request["SpawnAgent"] as? [String: Any])

    #expect(payload["session_id"] as? String == "session-1")
    #expect(payload["alias"] as? String == "reviewer")
    #expect(payload["provider"] as? String == "opencode")
    #expect(payload["model"] as? String == "default")
    #expect(payload["effort"] as? String == "high")
    #expect(payload["worktree_id"] is NSNull)
}

@Test func destroyAgentRequestMatchesKernelShape() throws {
    let data = try KernelProtocolCodec.encodeRequestFrame(
        KernelRequestFrame(
            requestID: "request-destroy",
            request: .destroyAgent(sessionID: "session-1", agentID: "agent-2")
        )
    )
    let object = try #require(
        JSONSerialization.jsonObject(with: data) as? [String: Any]
    )
    let request = try #require(object["request"] as? [String: Any])
    let payload = try #require(request["DestroyAgent"] as? [String: Any])

    #expect(payload["session_id"] as? String == "session-1")
    #expect(payload["agent_id"] as? String == "agent-2")
}

@Test func updateAgentConfigRequestMatchesKernelShape() throws {
    let data = try KernelProtocolCodec.encodeRequestFrame(
        KernelRequestFrame(
            requestID: "request-agent-config",
            request: .updateAgentConfig(
                sessionID: "session-1",
                agentID: "agent-2",
                executionMode: "plan",
                clearExecutionMode: false,
                permissionLevel: nil,
                clearPermissionLevel: true
            )
        )
    )
    let object = try #require(
        JSONSerialization.jsonObject(with: data) as? [String: Any]
    )
    let request = try #require(object["request"] as? [String: Any])
    let payload = try #require(request["UpdateAgentConfig"] as? [String: Any])

    #expect(payload["session_id"] as? String == "session-1")
    #expect(payload["agent_id"] as? String == "agent-2")
    #expect(payload["execution_mode"] as? String == "plan")
    #expect(payload["clear_execution_mode"] as? Bool == false)
    #expect(payload["permission_level"] is NSNull)
    #expect(payload["clear_permission_level"] as? Bool == true)
}

@Test func updateAgentProfileRequestMatchesKernelShape() throws {
    let data = try KernelProtocolCodec.encodeRequestFrame(
        KernelRequestFrame(
            requestID: "request-agent-profile",
            request: .updateAgentProfile(
                sessionID: "session-1",
                agentID: "agent-2",
                provider: "codex",
                model: "gpt-5.4",
                effort: nil,
                clearEffort: true
            )
        )
    )
    let object = try #require(
        JSONSerialization.jsonObject(with: data) as? [String: Any]
    )
    let request = try #require(object["request"] as? [String: Any])
    let payload = try #require(request["UpdateAgentProfile"] as? [String: Any])

    #expect(payload["session_id"] as? String == "session-1")
    #expect(payload["agent_id"] as? String == "agent-2")
    #expect(payload["provider"] as? String == "codex")
    #expect(payload["model"] as? String == "gpt-5.4")
    #expect(payload["effort"] is NSNull)
    #expect(payload["clear_effort"] as? Bool == true)
}

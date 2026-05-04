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

@Test func sessionsListedResponseDecodesRuntimeSessions() throws {
    let json = """
    {
      "type": "response",
      "request_id": "request-1",
      "response": {
        "SessionsListed": {
          "sessions": [
            {
              "id": "abc123456789",
              "alias": "demo",
              "workspace_id": "/repo",
              "worktree_id": "/repo",
              "status": "Active",
              "focused_agent_id": "agent-1",
              "created_at_ms": 1777111200000,
              "last_used_at_ms": 1777111201000,
              "agents": [
                {
                  "id": "agent-1",
                  "agent_ref": "1",
                  "alias": "builder",
                  "provider": "opencode",
                  "model": "default",
                  "state": "Focused",
                  "is_processing": false
                }
              ]
            }
          ]
        }
      },
      "error": null
    }
    """

    let frame = try KernelProtocolCodec.decodeResponseFrame(Data(json.utf8))
    guard case let .sessionsListed(sessions) = frame.response else {
        Issue.record("Expected SessionsListed response")
        return
    }

    #expect(frame.requestID == "request-1")
    #expect(sessions.count == 1)
    #expect(sessions[0].shortDisplayID == "demo")
    #expect(sessions[0].agents[0].alias == "builder")
}

@Test func sessionHistoryResponseDecodesEntries() throws {
    let json = """
    {
      "type": "response",
      "request_id": "request-history",
      "response": {
        "SessionHistory": {
          "entries": [
            {
              "entry_index": 1,
              "fragment_start": 0,
              "fragment_end": 5,
              "total_chars": 5,
              "entry": {
                "agent_id": "agent-1",
                "provider_run_id": "run-1",
                "kind": "provider_output",
                "merge_key": null,
                "text": "Hello"
              }
            }
          ],
          "next_cursor": null
        }
      },
      "error": null
    }
    """

    let frame = try KernelProtocolCodec.decodeResponseFrame(Data(json.utf8))
    guard case let .sessionHistory(entries) = frame.response else {
        Issue.record("Expected session history response")
        return
    }

    #expect(entries.count == 1)
    #expect(entries[0].entry.agentID == "agent-1")
    #expect(entries[0].entry.text == "Hello")
}

@Test func agentFocusedResponseDecodesFocusedAgent() throws {
    let json = """
    {
      "type": "response",
      "request_id": "request-focus",
      "response": {
        "AgentFocused": {
          "agent": {
            "id": "agent-2",
            "agent_ref": "2",
            "alias": "reviewer",
            "provider": "opencode",
            "model": "default",
            "state": "Focused",
            "is_processing": false
          }
        }
      },
      "error": null
    }
    """

    let frame = try KernelProtocolCodec.decodeResponseFrame(Data(json.utf8))
    guard case let .agentFocused(agent) = frame.response else {
        Issue.record("Expected AgentFocused response")
        return
    }

    #expect(agent?.id == "agent-2")
    #expect(agent?.displayName == "reviewer")
}

@Test func agentConfigUpdatedResponseDecodesAgentAndSession() throws {
    let json = """
    {
      "type": "response",
      "request_id": "request-agent-config",
      "response": {
        "AgentConfigUpdated": {
          "agent": {
            "id": "agent-2",
            "agent_ref": "2",
            "session_id": "session-1",
            "alias": "reviewer",
            "provider": "opencode",
            "model": "default",
            "effort": "high",
            "execution_mode_override": "plan",
            "permission_level_override": "required",
            "worktree_id": null,
            "state": "Focused",
            "is_processing": false
          },
          "session": {
            "id": "session-1",
            "alias": null,
            "workspace_id": "/repo",
            "worktree_id": "/repo",
            "status": "Active",
            "focused_agent_id": "agent-2",
            "created_at_ms": 1777111200000,
            "last_used_at_ms": 1777111201000,
            "agents": []
          }
        }
      },
      "error": null
    }
    """

    let frame = try KernelProtocolCodec.decodeResponseFrame(Data(json.utf8))
    guard case let .agentConfigUpdated(agent, session) = frame.response else {
        Issue.record("Expected AgentConfigUpdated response")
        return
    }

    #expect(agent.id == "agent-2")
    #expect(agent.executionModeOverride == "plan")
    #expect(agent.permissionLevelOverride == "required")
    #expect(session.focusedAgentID == "agent-2")
}

@Test func sessionConfigUpdatedResponseDecodesConfigAndSession() throws {
    let json = """
    {
      "type": "response",
      "request_id": "request-session-config",
      "response": {
        "SessionConfigUpdated": {
          "config": {
            "version": 1,
            "values": {
              "agents.mode": "plan"
            },
            "updated_by_attachment_id": "attachment-1"
          },
          "session": {
            "id": "session-1",
            "alias": null,
            "workspace_id": "/repo",
            "worktree_id": "/repo",
            "status": "Active",
            "config_state": {
              "version": 1,
              "values": {
                "agents.mode": "plan"
              },
              "updated_by_attachment_id": "attachment-1"
            },
            "focused_agent_id": null,
            "created_at_ms": 1777111200000,
            "last_used_at_ms": 1777111201000,
            "agents": []
          }
        }
      },
      "error": null
    }
    """

    let frame = try KernelProtocolCodec.decodeResponseFrame(Data(json.utf8))
    guard case let .sessionConfigUpdated(session, config) = frame.response else {
        Issue.record("Expected SessionConfigUpdated response")
        return
    }

    #expect(config.values["agents.mode"] == "plan")
    #expect(session.configState?.values["agents.mode"] == "plan")
}

@Test func promptSubmittedResponseAllowsMissingQueuedPromptState() throws {
    let json = """
    {
      "type": "response",
      "request_id": "request-submit",
      "response": {
        "PromptSubmitted": {
          "outcome": {
            "Started": {
              "prompt": {
                "id": "prompt-1",
                "source_attachment_id": "attachment-1",
                "target_agent_id": "agent-1",
                "prompt": "hello\\n",
                "status": "Running",
                "attachments": []
              }
            }
          },
          "session": {
            "id": "session-1",
            "alias": null,
            "workspace_id": "/repo",
            "worktree_id": "/repo",
            "status": "Active",
            "active_prompt": {
              "id": "prompt-1",
              "source_attachment_id": "attachment-1",
              "target_agent_id": "agent-1",
              "prompt": "hello\\n",
              "status": "Running",
              "attachments": []
            },
            "prompt_states": {
              "agent-1": {
                "active_prompt": {
                  "id": "prompt-1",
                  "source_attachment_id": "attachment-1",
                  "target_agent_id": "agent-1",
                  "prompt": "hello\\n",
                  "status": "Running",
                  "attachments": []
                }
              }
            },
            "focused_agent_id": "agent-1",
            "created_at_ms": 1777111200000,
            "last_used_at_ms": 1777111201000,
            "agents": []
          }
        }
      },
      "error": null
    }
    """

    let frame = try KernelProtocolCodec.decodeResponseFrame(Data(json.utf8))
    guard case let .promptSubmitted(session) = frame.response else {
        Issue.record("Expected PromptSubmitted response")
        return
    }

    #expect(session.activePrompt?.id == "prompt-1")
    #expect(session.promptStates["agent-1"]?.queuedPrompts == [])
}

@Test func providerCatalogResponseDecodesProvidersAndModels() throws {
    let json = """
    {
      "type": "response",
      "request_id": "request-provider-catalog",
      "response": {
        "ProviderCatalog": {
          "catalog": {
            "all": [
              {
                "id": "codex",
                "name": "Codex",
                "remote_machine_aliases": [],
                "models": {
                  "gpt-5.4": {
                    "id": "gpt-5.4",
                    "name": "GPT-5.4",
                    "status": "active",
                    "limit": {
                      "context": 200000,
                      "input": null,
                      "output": null
                    },
                    "variants": {
                      "medium": {}
                    }
                  }
                }
              }
            ],
            "default": {
              "codex": "gpt-5.4"
            },
            "connected": ["codex"]
          }
        }
      },
      "error": null
    }
    """

    let frame = try KernelProtocolCodec.decodeResponseFrame(Data(json.utf8))
    guard case let .providerCatalog(catalog) = frame.response else {
        Issue.record("Expected ProviderCatalog response")
        return
    }

    #expect(catalog.connected == ["codex"])
    #expect(catalog.default["codex"] == "gpt-5.4")
    #expect(catalog.all[0].models["gpt-5.4"]?.limit?.context == 200000)
    #expect(catalog.all[0].models["gpt-5.4"]?.variants.keys.contains("medium") == true)
}

@Test func providerAuthStatusResponseDecodesStatus() throws {
    let json = """
    {
      "type": "response",
      "request_id": "request-provider-auth",
      "response": {
        "ProviderAuthStatus": {
          "status": {
            "provider": "codex",
            "auth_state": "authenticated",
            "account_profile": "miguel",
            "login_hint": null,
            "detected_version": "1.2.3"
          }
        }
      },
      "error": null
    }
    """

    let frame = try KernelProtocolCodec.decodeResponseFrame(Data(json.utf8))
    guard case let .providerAuthStatus(status) = frame.response else {
        Issue.record("Expected ProviderAuthStatus response")
        return
    }

    #expect(status.provider == "codex")
    #expect(status.authState == "authenticated")
    #expect(status.accountProfile == "miguel")
    #expect(status.detectedVersion == "1.2.3")
}

@Test func mcpServersListedResponseDecodesServers() throws {
    let json = """
    {
      "type": "response",
      "request_id": "request-mcp-list",
      "response": {
        "McpServersListed": {
          "mcps": [
            {
              "name": "playwright",
              "transport": {
                "stdio": {
                  "command": "npx"
                }
              },
              "enabled": true,
              "required": false,
              "startup_timeout_sec": null,
              "tool_timeout_sec": null,
              "enabled_tools": null,
              "disabled_tools": null,
              "tools": null
            }
          ]
        }
      },
      "error": null
    }
    """

    let frame = try KernelProtocolCodec.decodeResponseFrame(Data(json.utf8))
    guard case let .mcpServersListed(mcps) = frame.response else {
        Issue.record("Expected McpServersListed response")
        return
    }

    #expect(mcps[0].name == "playwright")
    #expect(mcps[0].enabled == true)
    #expect(mcps[0].transport.keys.contains("stdio"))
}

@Test func skillsListedResponseDecodesSkills() throws {
    let json = """
    {
      "type": "response",
      "request_id": "request-skill-list",
      "response": {
        "SkillsListed": {
          "skills": [
            {
              "name": "swiftui-expert",
              "description": "SwiftUI guidance",
              "short_description": "SwiftUI",
              "path": "/skills/swiftui"
            }
          ]
        }
      },
      "error": null
    }
    """

    let frame = try KernelProtocolCodec.decodeResponseFrame(Data(json.utf8))
    guard case let .skillsListed(skills) = frame.response else {
        Issue.record("Expected SkillsListed response")
        return
    }

    #expect(skills[0].name == "swiftui-expert")
    #expect(skills[0].shortDescription == "SwiftUI")
}

@Test func sessionResponseDecodesPromptStateAndInteractions() throws {
    let json = """
    {
      "type": "response",
      "request_id": "request-session",
      "response": {
        "SessionState": {
          "session": {
            "id": "session-1",
            "alias": null,
            "workspace_id": "/repo",
            "worktree_id": "/repo",
            "status": "Active",
            "focused_agent_id": "agent-1",
            "active_prompt": {
              "id": "prompt-1",
              "source_attachment_id": "attachment-1",
              "target_agent_id": "agent-1",
              "prompt": "hello",
              "status": "Running"
            },
            "queued_prompts": [],
            "prompt_states": {
              "agent-1": {
                "active_prompt": {
                  "id": "prompt-1",
                  "source_attachment_id": "attachment-1",
                  "target_agent_id": "agent-1",
                  "prompt": "hello",
                  "status": "Running"
                },
                "queued_prompts": []
              }
            },
            "active_interactions": [
              {
                "id": "interaction-1",
                "agent_id": "agent-1",
                "kind": "permission",
                "level": "warning",
                "title": null,
                "message": "Allow command?",
                "choices": [
                  {
                    "id": "approve",
                    "label": "Approve",
                    "reply": "approved",
                    "style": "primary"
                  }
                ],
                "timeout_sec": null,
                "default_on_timeout": null,
                "requested_at_ms": 1777111201000
              }
            ],
            "created_at_ms": 1777111200000,
            "last_used_at_ms": 1777111201000,
            "agents": []
          }
        }
      },
      "error": null
    }
    """

    let frame = try KernelProtocolCodec.decodeResponseFrame(Data(json.utf8))
    guard case let .sessionState(session) = frame.response else {
        Issue.record("Expected SessionState response")
        return
    }

    #expect(session.activePrompt?.id == "prompt-1")
    #expect(session.promptStates["agent-1"]?.activePrompt?.status == "Running")
    #expect(session.activeInteractions.first?.choices.first?.style == "primary")
}

@Test func interactionRespondedResponseDecodesSession() throws {
    let json = """
    {
      "type": "response",
      "request_id": "request-interaction",
      "response": {
        "InteractionResponded": {
          "interaction_id": "interaction-1",
          "session": {
            "id": "session-1",
            "alias": null,
            "workspace_id": "/repo",
            "worktree_id": "/repo",
            "status": "Active",
            "focused_agent_id": null,
            "active_interactions": [],
            "created_at_ms": 1777111200000,
            "last_used_at_ms": 1777111201000,
            "agents": []
          }
        }
      },
      "error": null
    }
    """

    let frame = try KernelProtocolCodec.decodeResponseFrame(Data(json.utf8))
    guard case let .interactionResponded(interactionID, session) = frame.response else {
        Issue.record("Expected InteractionResponded response")
        return
    }

    #expect(interactionID == "interaction-1")
    #expect(session.activeInteractions.isEmpty)
}

@Test func eventFrameDecodesSessionSnapshot() throws {
    let json = """
    {
      "type": "event",
      "event_id": 7,
      "event": {
        "event": "session_snapshot",
        "provider_run": null,
        "session": {
          "id": "session-1",
          "alias": null,
          "workspace_id": "/repo",
          "worktree_id": "/repo",
          "status": "Active",
          "focused_agent_id": null,
          "created_at_ms": 1777111200000,
          "last_used_at_ms": 1777111201000,
          "agents": []
        }
      }
    }
    """

    let frame = try KernelProtocolCodec.decodeTransportFrame(Data(json.utf8))
    guard case let .event(eventFrame) = frame,
          case let .sessionSnapshot(session) = eventFrame.event
    else {
        Issue.record("Expected session snapshot event")
        return
    }

    #expect(eventFrame.eventID == 7)
    #expect(session.id == "session-1")
    #expect(session.workspaceID == "/repo")
}

@Test func eventFrameDecodesTerminalOutput() throws {
    let json = """
    {
      "type": "event",
      "event_id": 8,
      "event": {
        "event": "terminal_output",
        "records": [
          {
            "agent_id": "agent-1",
            "kind": "provider_output",
            "merge_key": null,
            "bytes": [72, 101, 108, 108, 111]
          }
        ]
      }
    }
    """

    let frame = try KernelProtocolCodec.decodeTransportFrame(Data(json.utf8))
    guard case let .event(eventFrame) = frame,
          case let .terminalOutput(records) = eventFrame.event
    else {
        Issue.record("Expected terminal output event")
        return
    }

    #expect(eventFrame.eventID == 8)
    #expect(records[0].agentID == "agent-1")
    #expect(records[0].text == "Hello")
}

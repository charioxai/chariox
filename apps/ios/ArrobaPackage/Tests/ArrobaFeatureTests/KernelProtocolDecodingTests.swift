import Foundation
import Testing
@testable import ArrobaFeature

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

@Test func agentProfileUpdatedResponseDecodesAgentAndSession() throws {
    let json = """
    {
      "type": "response",
      "request_id": "request-agent-profile",
      "response": {
        "AgentProfileUpdated": {
          "agent": {
            "id": "agent-2",
            "agent_ref": "2",
            "session_id": "session-1",
            "alias": "reviewer",
            "provider": "codex",
            "model": "gpt-5.4",
            "effort": "low",
            "execution_mode_override": null,
            "permission_level_override": null,
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
    guard case let .agentProfileUpdated(agent, session) = frame.response else {
        Issue.record("Expected AgentProfileUpdated response")
        return
    }

    #expect(agent.id == "agent-2")
    #expect(agent.provider == "codex")
    #expect(agent.model == "gpt-5.4")
    #expect(agent.effort == "low")
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

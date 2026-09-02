use super::*;

#[cfg(test)]
mod workspace_live_sync_tests {
    use super::*;

    #[test]
    fn workspace_live_sync_specs_expose_read_and_edit_tools() {
        let specs = workspace_live_sync_runtime_tool_specs();
        assert!(specs.iter().any(|spec| spec.name == READ_ARTIFACT_TOOL));
        assert!(specs
            .iter()
            .any(|spec| spec.name == READ_ARTIFACT_TOOL_ALIAS));
        assert!(specs.iter().any(|spec| spec.name == EDIT_ARTIFACT_TOOL));
        assert!(specs
            .iter()
            .any(|spec| spec.name == EDIT_ARTIFACT_TOOL_ALIAS));
        assert!(!specs.iter().any(|spec| spec.name == APPLY_PATCH_TOOL));
        assert!(specs
            .iter()
            .any(|spec| spec.name == PATCH_ARTIFACT_TOOL_ALIAS));
        assert!(!specs.iter().any(|spec| spec.name == APPLY_PATCH_TOOL_ALIAS));
        assert!(specs.iter().any(|spec| spec.name == WRITE_ARTIFACT_TOOL));
        assert!(specs
            .iter()
            .any(|spec| spec.name == WRITE_ARTIFACT_TOOL_ALIAS));
        assert!(specs.iter().any(|spec| spec.name == DELETE_ARTIFACT_TOOL));
        assert!(specs
            .iter()
            .any(|spec| spec.name == DELETE_ARTIFACT_TOOL_ALIAS));
        assert!(specs.iter().any(|spec| spec.name == MOVE_ARTIFACT_TOOL));
        assert!(specs
            .iter()
            .any(|spec| spec.name == MOVE_ARTIFACT_TOOL_ALIAS));
    }

    #[test]
    fn extension_specs_expose_discovery_and_request_tools() {
        let specs = extension_runtime_tool_specs();
        assert!(specs.iter().any(|spec| spec.name == LIST_EXTENSIONS_TOOL));
        assert!(specs.iter().any(|spec| spec.name == REQUEST_EXTENSION_TOOL));
        assert!(specs.iter().any(|spec| spec.name == REGISTER_MCP_TOOL));
        assert!(specs
            .iter()
            .any(|spec| spec.name == REGISTER_SKILL_PATH_TOOL));
        assert!(specs
            .iter()
            .any(|spec| spec.name == REGISTER_ENVIRONMENT_TOOL));
        assert!(specs
            .iter()
            .any(|spec| spec.name == REGISTER_SCRIPT_PATH_TOOL));
        assert!(specs
            .iter()
            .any(|spec| spec.name == REGISTER_CONNECTOR_PATH_TOOL));
        assert!(specs
            .iter()
            .any(|spec| spec.name == REGISTER_CONNECTOR_ADAPTER_PATH_TOOL));
        assert!(!specs.iter().any(|spec| spec.name == "list_extensions"));
        assert!(!specs.iter().any(|spec| spec.name == "request_extension"));
    }

    #[test]
    fn recall_specs_expose_search_and_query_tools_without_compat_aliases() {
        let specs = recall_runtime_tool_specs();
        assert!(specs.iter().any(|spec| spec.name == SEARCH_RECALL_TOOL));
        assert!(specs.iter().any(|spec| spec.name == QUERY_RECALL_TOOL));
        assert!(!specs.iter().any(|spec| spec.name == "search_recall"));
        assert!(!specs.iter().any(|spec| spec.name == "query_recall"));
    }

    #[test]
    fn workflow_specs_expose_agent_app_action_tool() {
        let specs = workflow_runtime_tool_specs();
        assert!(specs
            .iter()
            .any(|spec| spec.name == AGENT_APP_ACTION_TOOL_QUALIFIED));
        assert_eq!(
            canonical_workflow_tool_name("mcp__chariox__chariox_agent_app_action"),
            Some(AGENT_APP_ACTION_TOOL)
        );
    }

    #[test]
    fn ordinary_workflow_specs_omit_opt_in_event_reply_tool() {
        let specs = workflow_runtime_tool_specs_without_event_reply();
        assert!(!specs
            .iter()
            .any(|spec| spec.name == REPLY_TO_EVENT_TOOL_QUALIFIED));
        assert!(!specs
            .iter()
            .any(|spec| spec.name == EVENT_CONTEXT_TOOL_QUALIFIED));
        assert!(!specs
            .iter()
            .any(|spec| spec.name == EVENT_ACTION_TOOL_QUALIFIED));
        assert_eq!(
            workflow_reply_to_event_tool_spec().name,
            REPLY_TO_EVENT_TOOL_QUALIFIED
        );
        assert_eq!(
            canonical_workflow_tool_name("mcp__chariox__event_context"),
            Some(EVENT_CONTEXT_TOOL)
        );
        assert_eq!(
            workflow_event_action_tool_spec().name,
            EVENT_ACTION_TOOL_QUALIFIED
        );
        assert_eq!(
            canonical_workflow_tool_name("mcp__chariox__event_action"),
            Some(EVENT_ACTION_TOOL)
        );
    }

    #[test]
    fn intermediate_workflow_output_tool_spec_describes_user_visible_multi_emit_channel() {
        let specs = workflow_runtime_tool_specs();
        let spec = specs
            .iter()
            .find(|spec| spec.name == VALIDATE_AND_SUBMIT_INTERMEDIATE_WORKFLOW_RUN_OUTPUT_TOOL)
            .expect("intermediate workflow output tool spec should exist");

        assert!(spec
            .description
            .contains("one user-visible intermediate workflow output event"));
        assert!(spec
            .description
            .contains("may be called multiple times in one workflow node turn"));
        assert!(spec
            .description
            .contains("does not send data to downstream nodes"));
    }

    #[test]
    fn meta_event_specs_describe_visible_event_prompts() {
        let specs = meta_runtime_tool_specs();
        let list_events = specs
            .iter()
            .find(|spec| spec.name == META_LIST_EVENTS_TOOL)
            .expect("meta list events tool should be exposed");
        assert!(list_events.description.contains("visible runtime prompts"));
        assert!(!list_events.description.contains("prompt injection"));
    }

    #[test]
    fn meta_specs_expose_guide_tools() {
        let specs = meta_runtime_tool_specs();
        assert!(specs
            .iter()
            .any(|spec| spec.name == META_SEARCH_GUIDES_TOOL));
        assert!(specs.iter().any(|spec| spec.name == META_LIST_GUIDES_TOOL));
        assert!(specs.iter().any(|spec| spec.name == META_READ_GUIDE_TOOL));
        assert_eq!(
            canonical_meta_tool_name("mcp__chariox__chariox_meta_read_guide"),
            Some(META_READ_GUIDE_TOOL)
        );
    }

    #[test]
    fn meta_specs_expose_workflow_code_tools() {
        let specs = meta_runtime_tool_specs();
        assert!(specs
            .iter()
            .any(|spec| spec.name == META_WORKFLOW_CODE_CREATE_TOOL));
        assert!(specs
            .iter()
            .any(|spec| spec.name == META_WORKFLOW_CODE_READ_TOOL));
        assert!(specs
            .iter()
            .any(|spec| spec.name == META_WORKFLOW_CODE_LIST_TOOL));
        assert!(specs
            .iter()
            .any(|spec| spec.name == META_WORKFLOW_CODE_UPDATE_TOOL));
        assert!(specs
            .iter()
            .any(|spec| spec.name == META_WORKFLOW_CODE_DELETE_TOOL));
        assert!(specs
            .iter()
            .any(|spec| spec.name == META_WORKFLOW_CODE_VALIDATE_TOOL));
        assert!(specs
            .iter()
            .any(|spec| spec.name == META_WORKFLOW_CODE_APPLY_TOOL));
        assert!(specs
            .iter()
            .any(|spec| spec.name == META_WORKFLOW_CODE_RUN_TOOL));
        assert!(specs
            .iter()
            .any(|spec| spec.name == META_WORKFLOW_CODE_EXPORT_TOOL));
        assert!(specs
            .iter()
            .any(|spec| spec.name == META_WORKFLOW_CODE_IMPORT_TOOL));
        assert!(specs
            .iter()
            .any(|spec| spec.name == META_WORKFLOW_CODE_PACKAGE_EXPORT_TOOL));
        assert!(specs
            .iter()
            .any(|spec| spec.name == META_WORKFLOW_CODE_PACKAGE_IMPORT_TOOL));
        assert!(specs
            .iter()
            .any(|spec| spec.name == META_WORKFLOW_CODE_SOURCE_EXPORT_TOOL));
        assert!(specs
            .iter()
            .any(|spec| spec.name == META_WORKFLOW_CODE_SOURCE_EXPORT_DIRECTORY_TOOL));
        assert!(specs
            .iter()
            .any(|spec| spec.name == META_WORKFLOW_CODE_CANVAS_CONTRACT_TOOL));
        assert_eq!(
            canonical_meta_tool_name("mcp__chariox__chariox_meta_workflow_code_create"),
            Some(META_WORKFLOW_CODE_CREATE_TOOL)
        );
        assert_eq!(
            canonical_meta_tool_name("mcp__chariox__meta_workflow_code_list"),
            Some(META_WORKFLOW_CODE_LIST_TOOL)
        );
        assert_eq!(
            canonical_meta_tool_name("mcp__chariox__meta_workflow_code_delete"),
            Some(META_WORKFLOW_CODE_DELETE_TOOL)
        );
        assert_eq!(
            canonical_meta_tool_name("mcp__chariox__meta_workflow_code_apply"),
            Some(META_WORKFLOW_CODE_APPLY_TOOL)
        );
        assert_eq!(
            canonical_meta_tool_name("mcp__chariox__meta_workflow_code_run"),
            Some(META_WORKFLOW_CODE_RUN_TOOL)
        );
        assert_eq!(
            canonical_meta_tool_name("mcp__chariox__meta_workflow_code_export"),
            Some(META_WORKFLOW_CODE_EXPORT_TOOL)
        );
        assert_eq!(
            canonical_meta_tool_name("mcp__chariox__meta_workflow_code_import"),
            Some(META_WORKFLOW_CODE_IMPORT_TOOL)
        );
        assert_eq!(
            canonical_meta_tool_name("mcp__chariox__meta_workflow_code_package_export"),
            Some(META_WORKFLOW_CODE_PACKAGE_EXPORT_TOOL)
        );
        assert_eq!(
            canonical_meta_tool_name("mcp__chariox__meta_workflow_code_package_import"),
            Some(META_WORKFLOW_CODE_PACKAGE_IMPORT_TOOL)
        );
        assert_eq!(
            canonical_meta_tool_name("mcp__chariox__meta_workflow_code_source_export"),
            Some(META_WORKFLOW_CODE_SOURCE_EXPORT_TOOL)
        );
        assert_eq!(
            canonical_meta_tool_name("mcp__chariox__meta_workflow_code_source_export_directory"),
            Some(META_WORKFLOW_CODE_SOURCE_EXPORT_DIRECTORY_TOOL)
        );
        assert_eq!(
            canonical_meta_tool_name("mcp__chariox__meta_workflow_code_source_export_dir"),
            Some(META_WORKFLOW_CODE_SOURCE_EXPORT_DIRECTORY_TOOL)
        );
        assert_eq!(
            canonical_meta_tool_name("mcp__chariox__meta_workflow_code_canvas_contract"),
            Some(META_WORKFLOW_CODE_CANVAS_CONTRACT_TOOL)
        );
    }

    #[test]
    fn workflow_code_meta_specs_advertise_languages_and_provider_rebindings() {
        let specs = meta_runtime_tool_specs();
        for tool_name in [
            META_WORKFLOW_CODE_CREATE_TOOL,
            META_WORKFLOW_CODE_UPDATE_TOOL,
            META_WORKFLOW_CODE_VALIDATE_TOOL,
            META_WORKFLOW_CODE_APPLY_TOOL,
            META_WORKFLOW_CODE_RUN_TOOL,
        ] {
            let spec = specs
                .iter()
                .find(|spec| spec.name == tool_name)
                .expect("workflow-code tool should exist");
            assert!(
                spec.input_schema
                    .pointer("/properties/language/enum")
                    .and_then(serde_json::Value::as_array)
                    .is_some_and(|values| values
                        .iter()
                        .any(|value| value.as_str() == Some("javascript"))),
                "{tool_name} should advertise canonical JavaScript language"
            );
            assert!(
                spec.input_schema
                    .pointer("/properties/language/enum")
                    .and_then(serde_json::Value::as_array)
                    .is_some_and(|values| values
                        .iter()
                        .any(|value| value.as_str() == Some("typescript"))),
                "{tool_name} should advertise canonical TypeScript language"
            );
        }

        let apply = specs
            .iter()
            .find(|spec| spec.name == META_WORKFLOW_CODE_APPLY_TOOL)
            .expect("workflow-code apply tool should exist");
        assert_eq!(
            apply
                .input_schema
                .pointer("/properties/provider_rebindings/items/properties/account_profile/type"),
            Some(&serde_json::json!("string"))
        );

        let canvas_contract = specs
            .iter()
            .find(|spec| spec.name == META_WORKFLOW_CODE_CANVAS_CONTRACT_TOOL)
            .expect("workflow-code canvas contract tool should exist");
        assert!(canvas_contract.description.contains("canvas dimensions"));
        assert_eq!(
            canvas_contract
                .input_schema
                .pointer("/additionalProperties"),
            Some(&serde_json::json!(false))
        );
    }

    #[test]
    fn slice_specs_expose_screen_input_and_ocr_tools() {
        let specs = slice_runtime_tool_specs();
        assert!(specs
            .iter()
            .any(|spec| spec.name == SLICE_SCREEN_STATUS_TOOL));
        assert!(specs
            .iter()
            .any(|spec| spec.name == SLICE_SCREEN_STATUS_TOOL_ALIAS));
        assert!(specs.iter().any(|spec| spec.name == SLICE_SCREENSHOT_TOOL));
        assert!(specs
            .iter()
            .any(|spec| spec.name == SLICE_SCREENSHOT_TOOL_ALIAS));
        assert!(specs.iter().any(|spec| spec.name == SLICE_OCR_TOOL));
        assert!(specs.iter().any(|spec| spec.name == SLICE_FIND_TEXT_TOOL));
        assert!(specs.iter().any(|spec| spec.name == SLICE_MOUSE_TOOL));
        assert!(specs.iter().any(|spec| spec.name == SLICE_KEYBOARD_TOOL));
        assert!(specs.iter().any(|spec| spec.name == SLICE_OPEN_URL_TOOL));
        assert!(specs
            .iter()
            .any(|spec| spec.name == SLICE_BROWSER_STATUS_TOOL));
        assert!(specs
            .iter()
            .any(|spec| spec.name == SLICE_BROWSER_STATUS_TOOL_ALIAS));
        assert!(specs
            .iter()
            .any(|spec| spec.name == SLICE_BROWSER_FIND_TOOL));
        assert!(specs
            .iter()
            .any(|spec| spec.name == SLICE_BROWSER_FILL_TOOL));
        assert!(specs
            .iter()
            .any(|spec| spec.name == SLICE_BROWSER_CLICK_TOOL));
        assert!(specs
            .iter()
            .any(|spec| spec.name == SLICE_BROWSER_SUBMIT_TOOL));
        assert!(specs
            .iter()
            .any(|spec| spec.name == SLICE_BROWSER_DIALOG_TOOL));
        assert!(specs
            .iter()
            .any(|spec| spec.name == SLICE_BROWSER_DIALOG_TOOL_ALIAS));
        for name in [
            "chariox.slice_browser_events",
            "slice_browser_events",
            "chariox.slice_browser_downloads",
            "slice_browser_downloads",
            "chariox.slice_browser_upload",
            "slice_browser_upload",
            "chariox.slice_browser_permission",
            "slice_browser_permission",
        ] {
            assert!(
                specs.iter().any(|spec| spec.name == name),
                "missing runtime browser tool {name}"
            );
        }
        let permission = specs
            .iter()
            .find(|spec| spec.name == SLICE_BROWSER_PERMISSION_TOOL)
            .expect("browser permission tool spec");
        assert_eq!(
            permission.input_schema["properties"]["permission"]["enum"],
            serde_json::json!([
                "camera",
                "clipboard-read-write",
                "clipboard-sanitized-write",
                "display-capture",
                "geolocation",
                "local-fonts",
                "microphone",
                "midi",
                "midi-sysex",
                "notifications"
            ])
        );
        assert!(specs
            .iter()
            .any(|spec| spec.name == SLICE_BROWSER_TEXT_TOOL));
        assert!(specs
            .iter()
            .any(|spec| spec.name == SLICE_BROWSER_WAIT_FOR_TEXT_TOOL));
        assert!(specs
            .iter()
            .any(|spec| spec.name == SLICE_BROWSER_WAIT_FOR_TEXT_TOOL_ALIAS));
        assert!(specs
            .iter()
            .any(|spec| spec.name == SLICE_BROWSER_WAIT_FOR_SELECTOR_TOOL));
        assert!(specs
            .iter()
            .any(|spec| spec.name == SLICE_BROWSER_WAIT_FOR_IDLE_TOOL));
    }

    #[test]
    fn credential_specs_expose_browser_and_computer_secret_paste_tools() {
        let specs = credential_runtime_tool_specs();
        assert!(specs
            .iter()
            .any(|spec| spec.name == PASTE_SECRET_TO_SLICE_TOOL));
        assert!(specs
            .iter()
            .any(|spec| spec.name == PASTE_SECRET_TO_SLICE_TOOL_ALIAS));
        assert!(specs
            .iter()
            .any(|spec| spec.name == PASTE_SECRET_TO_COMPUTER_TOOL));
        assert!(specs
            .iter()
            .any(|spec| spec.name == PASTE_SECRET_TO_COMPUTER_TOOL_ALIAS));
        assert!(specs
            .iter()
            .any(|spec| spec.name == CREATE_GENERATED_CREDENTIAL_TOOL));
        assert!(specs
            .iter()
            .any(|spec| spec.name == CREATE_GENERATED_CREDENTIAL_TOOL_ALIAS));
        assert!(specs
            .iter()
            .any(|spec| spec.name == REQUEST_CREDENTIAL_SECRET_TOOL));
        assert!(specs
            .iter()
            .any(|spec| spec.name == REQUEST_CREDENTIAL_SECRET_TOOL_ALIAS));
        let create = specs
            .iter()
            .find(|spec| spec.name == CREATE_GENERATED_CREDENTIAL_TOOL)
            .expect("generated credential tool spec");
        assert!(
            create.input_schema["properties"]["credential"]["properties"]["allowed_uses"]["items"]
                ["enum"]
                .as_array()
                .is_some_and(|uses| uses.contains(&serde_json::json!("computer")))
        );
        assert!(
            create.input_schema["properties"]["credential"]["properties"]["injection"]
                ["properties"]["kind"]["enum"]
                .as_array()
                .is_some_and(|kinds| kinds.contains(&serde_json::json!("computer")))
        );
    }

    #[test]
    fn controller_browser_tool_arguments_are_closed_bounded_and_path_redacted() {
        let events: SliceBrowserEventsArgs = serde_json::from_value(serde_json::json!({
            "browser_generation": 7
        }))
        .expect("minimal event poll arguments");
        assert_eq!(events.cursor, 0);
        assert_eq!(events.limit, 100);
        assert!(
            serde_json::from_value::<SliceBrowserEventsArgs>(serde_json::json!({
                "browser_generation": 7,
                "unexpected": true
            }))
            .is_err()
        );

        let upload: SliceBrowserUploadArgs = serde_json::from_value(serde_json::json!({
            "field_id": "element-1",
            "files": ["/private/must-not-appear.txt"]
        }))
        .expect("bounded upload arguments");
        let debug = format!("{upload:?}");
        assert!(debug.contains("file_count"));
        assert!(!debug.contains("must-not-appear"));
    }

    #[test]
    fn canonical_extension_tool_name_accepts_provider_aliases() {
        assert_eq!(
            canonical_extension_tool_name("mcp__chariox__list_extensions"),
            Some(LIST_EXTENSIONS_TOOL)
        );
        assert_eq!(
            canonical_extension_tool_name("mcp__chariox__chariox_request_extension"),
            Some(REQUEST_EXTENSION_TOOL)
        );
        assert_eq!(
            canonical_extension_tool_name("mcp__chariox__register_mcp"),
            Some(REGISTER_MCP_TOOL)
        );
        assert_eq!(
            canonical_extension_tool_name("mcp__chariox__chariox_register_connector_adapter_path"),
            Some(REGISTER_CONNECTOR_ADAPTER_PATH_TOOL)
        );
        assert_eq!(canonical_extension_tool_name("unknown"), None);
    }

    #[test]
    fn canonical_recall_tool_name_accepts_provider_aliases() {
        assert_eq!(
            canonical_recall_tool_name("mcp__chariox__search_recall"),
            Some(SEARCH_RECALL_TOOL)
        );
        assert_eq!(
            canonical_recall_tool_name("mcp__chariox__chariox_query_recall"),
            Some(QUERY_RECALL_TOOL)
        );
        assert_eq!(canonical_recall_tool_name("unknown"), None);
    }

    #[test]
    fn canonical_slice_tool_name_accepts_provider_aliases() {
        assert_eq!(
            canonical_slice_tool_name("mcp__chariox__slice_screenshot"),
            Some(SLICE_SCREENSHOT_TOOL)
        );
        assert_eq!(
            canonical_slice_tool_name("mcp__chariox__chariox_slice_mouse"),
            Some(SLICE_MOUSE_TOOL)
        );
        assert_eq!(
            canonical_slice_tool_name("slice_open_url"),
            Some(SLICE_OPEN_URL_TOOL)
        );
        assert_eq!(
            canonical_slice_tool_name("mcp__chariox__slice_browser_fill"),
            Some(SLICE_BROWSER_FILL_TOOL)
        );
        assert_eq!(
            canonical_slice_tool_name("slice_browser_status"),
            Some(SLICE_BROWSER_STATUS_TOOL)
        );
        assert_eq!(
            canonical_slice_tool_name("mcp__chariox__slice_browser_wait_for_text"),
            Some(SLICE_BROWSER_WAIT_FOR_TEXT_TOOL)
        );
        assert_eq!(
            canonical_slice_tool_name("slice_browser_wait_for_idle"),
            Some(SLICE_BROWSER_WAIT_FOR_IDLE_TOOL)
        );
        assert_eq!(
            canonical_slice_tool_name("mcp__chariox__slice_browser_dialog"),
            Some(SLICE_BROWSER_DIALOG_TOOL)
        );
        assert_eq!(
            canonical_slice_tool_name("mcp__chariox__slice_browser_events"),
            Some("chariox.slice_browser_events")
        );
        assert_eq!(
            canonical_slice_tool_name("slice_browser_downloads"),
            Some("chariox.slice_browser_downloads")
        );
        assert_eq!(
            canonical_slice_tool_name("mcp__chariox__slice_browser_upload"),
            Some("chariox.slice_browser_upload")
        );
        assert_eq!(
            canonical_slice_tool_name("slice_browser_permission"),
            Some("chariox.slice_browser_permission")
        );
        assert_eq!(canonical_slice_tool_name("unknown"), None);
    }

    #[test]
    fn canonical_credential_tool_name_accepts_browser_paste_aliases() {
        assert_eq!(
            canonical_credential_tool_name("mcp__chariox__chariox_create_generated_credential"),
            Some(CREATE_GENERATED_CREDENTIAL_TOOL)
        );
        assert_eq!(
            canonical_credential_tool_name("request_credential_secret"),
            Some(REQUEST_CREDENTIAL_SECRET_TOOL)
        );
        assert_eq!(
            canonical_credential_tool_name("paste_secret_to_slice"),
            Some(PASTE_SECRET_TO_SLICE_TOOL)
        );
        assert_eq!(
            canonical_credential_tool_name("mcp__chariox__paste_secret_to_slice"),
            Some(PASTE_SECRET_TO_SLICE_TOOL)
        );
        assert_eq!(
            canonical_credential_tool_name("paste_secret_to_computer"),
            Some(PASTE_SECRET_TO_COMPUTER_TOOL)
        );
        assert_eq!(
            canonical_credential_tool_name("mcp__chariox__paste_secret_to_computer"),
            Some(PASTE_SECRET_TO_COMPUTER_TOOL)
        );
        assert_eq!(
            canonical_credential_tool_name("manage_credential_vault"),
            Some(MANAGE_CREDENTIAL_VAULT_TOOL)
        );
        assert_eq!(canonical_credential_tool_name("unknown"), None);
    }

    #[test]
    fn canonical_workspace_live_sync_tool_name_accepts_provider_aliases() {
        assert_eq!(
            canonical_workspace_live_sync_tool_name("mcp__chariox__read_artifact"),
            Some(READ_ARTIFACT_TOOL)
        );
        assert_eq!(
            canonical_workspace_live_sync_tool_name("mcp__chariox__chariox_read_artifact"),
            Some(READ_ARTIFACT_TOOL)
        );
        assert_eq!(
            canonical_workspace_live_sync_tool_name("read_artifact"),
            Some(READ_ARTIFACT_TOOL)
        );
        assert_eq!(
            canonical_workspace_live_sync_tool_name("edit_artifact"),
            Some(EDIT_ARTIFACT_TOOL)
        );
        assert_eq!(
            canonical_workspace_live_sync_tool_name("patch_artifact"),
            Some(APPLY_PATCH_TOOL)
        );
        assert_eq!(
            canonical_workspace_live_sync_tool_name("chariox_apply_patch"),
            Some(APPLY_PATCH_TOOL)
        );
        assert_eq!(
            canonical_workspace_live_sync_tool_name("chariox_patch_artifact"),
            Some(APPLY_PATCH_TOOL)
        );
        assert_eq!(canonical_workspace_live_sync_tool_name("unknown"), None);
    }

    #[test]
    fn managed_edit_args_accept_text_replace_shape() {
        let args = serde_json::from_value::<WorkspaceLiveSyncEditArtifactArgs>(serde_json::json!({
            "path": "src/lib.rs",
            "snapshot_id": "snap:1",
            "old_text": "before",
            "new_text": "after"
        }))
        .expect("workspace live sync edit args should parse");

        assert_eq!(args.path, "src/lib.rs");
        assert_eq!(args.old_text.as_deref(), Some("before"));
        assert_eq!(args.new_text, "after");
    }

    #[test]
    fn managed_write_args_accept_text_content_shape() {
        let args =
            serde_json::from_value::<WorkspaceLiveSyncWriteArtifactArgs>(serde_json::json!({
                "path": "src/lib.rs",
                "content_text": "hello"
            }))
            .expect("workspace live sync write args should parse");

        assert_eq!(args.path, "src/lib.rs");
        assert_eq!(args.content_text.as_deref(), Some("hello"));
        assert_eq!(args.content_base64, None);
    }

    #[test]
    fn managed_write_args_accept_opaque_content_shape() {
        let args =
            serde_json::from_value::<WorkspaceLiveSyncWriteArtifactArgs>(serde_json::json!({
                "path": "assets/blob.bin",
                "content_base64": "AAEC",
                "domain": "opaque"
            }))
            .expect("managed opaque write args should parse");

        assert_eq!(args.path, "assets/blob.bin");
        assert_eq!(args.content_text, None);
        assert_eq!(args.content_base64.as_deref(), Some("AAEC"));
    }

    #[test]
    fn managed_apply_patch_args_accept_patch_text_shape() {
        let args = serde_json::from_value::<WorkspaceLiveSyncApplyPatchArgs>(serde_json::json!({
            "patch_text": "*** Begin Patch\n*** Update File: src/lib.rs\n@@\n-old\n+new\n*** End Patch",
            "domain": "text"
        }))
        .expect("managed apply patch args should parse");

        assert!(args.patch_text.contains("*** Begin Patch"));
        assert_eq!(args.domain.as_deref(), Some("text"));
    }

    #[test]
    fn managed_delete_args_accept_path_shape() {
        let args =
            serde_json::from_value::<WorkspaceLiveSyncDeleteArtifactArgs>(serde_json::json!({
                "path": "src/lib.rs"
            }))
            .expect("managed delete args should parse");

        assert_eq!(args.path, "src/lib.rs");
    }

    #[test]
    fn managed_move_args_accept_path_shape() {
        let args = serde_json::from_value::<WorkspaceLiveSyncMoveArtifactArgs>(serde_json::json!({
            "from_path": "src/old.rs",
            "to_path": "src/new.rs",
            "old_text": "old",
            "new_text": "new"
        }))
        .expect("managed move args should parse");

        assert_eq!(args.from_path, "src/old.rs");
        assert_eq!(args.to_path, "src/new.rs");
        assert_eq!(args.old_text.as_deref(), Some("old"));
        assert_eq!(args.new_text.as_deref(), Some("new"));
    }

    #[test]
    fn managed_move_args_treat_empty_transform_fields_as_absent_for_non_text() {
        let args = serde_json::from_value::<WorkspaceLiveSyncMoveArtifactArgs>(serde_json::json!({
            "from_path": "from.bin",
            "to_path": "to.bin",
            "old_text": "",
            "new_text": "",
            "domain": "opaque"
        }))
        .expect("managed move args should parse");

        assert!(!args.has_non_text_transform_fields());
    }

    #[test]
    fn managed_move_args_treat_empty_transform_pair_as_absent_for_text() {
        let args = serde_json::from_value::<WorkspaceLiveSyncMoveArtifactArgs>(serde_json::json!({
            "from_path": "from.txt",
            "to_path": "to.txt",
            "old_text": "",
            "new_text": "",
            "domain": "text"
        }))
        .expect("managed move args should parse");

        assert_eq!(args.normalized_text_transform_fields(), (None, None));
    }

    #[test]
    fn generic_json_output_schema_validator_accepts_valid_output() {
        let schema = serde_json::json!({
            "type": "object",
            "required": ["answer"],
            "additionalProperties": false,
            "properties": {
                "answer": {"type": "string"}
            }
        });
        validate_json_output_schema("test_schema", &schema, r#"{"answer":"ok"}"#)
            .expect("valid output should pass schema validation");
    }

    #[test]
    fn generic_json_output_schema_validator_reports_invalid_output() {
        let schema = serde_json::json!({
            "type": "object",
            "required": ["answer"],
            "additionalProperties": false,
            "properties": {
                "answer": {"type": "string"}
            }
        });
        let error = validate_json_output_schema("test_schema", &schema, r#"{"extra":true}"#)
            .expect_err("invalid output should fail schema validation");
        assert!(error.contains("required") || error.contains("Additional properties"));
    }

    #[test]
    fn workflow_handoff_schema_validator_keeps_schema_ref_contract() {
        let path = std::env::temp_dir().join(format!(
            "chariox-workflow-schema-{}.json",
            crate::session::unix_epoch_ms()
        ));
        std::fs::write(
            &path,
            r#"{"type":"object","required":["status"],"properties":{"status":{"type":"string"}},"additionalProperties":false}"#,
        )
        .expect("schema file should be writable");
        validate_workflow_handoff_schema(
            path.to_str().expect("schema path should be utf8"),
            r#"{"status":"ok"}"#,
        )
        .expect("workflow handoff schema-ref validation should still pass");
        let _ = std::fs::remove_file(path);
    }
}

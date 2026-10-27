// MP-11: opt-in diagnostics identify public code, never exception/page values.
const names=new Set(['Error','TypeError','RangeError','ReferenceError','SyntaxError']);
const files=new Set(['linux-owned-desktop.mjs','native-keyboard-channel.mjs','kernel-browser-host.mjs','kernel-browser-process.mjs','kernel-browser-owned-display.mjs','kernel-browser-linux.mjs','kernel-browser-display.mjs','kernel-browser-motion.mjs','kernel-browser-native.mjs','kernel-browser-native-worker.mjs','kernel-browser-native-pipe.mjs','kernel-browser-native-credit.mjs','kernel-browser-refiner.mjs','kernel-browser-pixels.mjs','kernel-browser-pixel-worker.mjs','kernel-browser-display-capture.mjs','kernel-browser-display-credit.mjs','browser-controller-cdp.mjs','browser-controller-actions.mjs','browser-controller-compatibility.mjs','kernel-browser-mirror2.mjs','native-computer.mjs','browser-protection-regions.mjs']);
// MP-08/MP-10/MP-11: region adapters are public source positions, not page exception text.
for(const file of ['kernel-browser-resize.mjs','kernel-browser-display-document.mjs','kernel-browser-mirror2-frames.mjs','kernel-browser-input.mjs','kernel-browser-page-display.mjs','kernel-browser-visible-page.mjs','kernel-browser-popup-evidence.mjs','browser-controller-notes.mjs','kernel-browser-window-region.mjs','kernel-browser-window-display.mjs','kernel-browser-display-regions.mjs','kernel-browser-chrome-window.mjs','kernel-browser-native-viewport.mjs','kernel-browser-native-readback.mjs','kernel-browser-native-windows.mjs','kernel-browser-webcodecs.mjs','browser-focus-world.mjs','browser-controller-frames.mjs','browser-controller-snapshot.mjs','browser-controller-input.mjs','kernel-browser-input-queue.mjs','kernel-browser-mirror.mjs','kernel-browser-mirror2-closed.mjs','kernel-browser-mirror2-css.mjs','kernel-browser-mirror2-regions.mjs','kernel-browser-native-tab-state.mjs'])files.add(file);
const codes=new Set(['ENOENT','EACCES','EINVAL','EPIPE','ETIMEDOUT','ERR_BUFFER_OUT_OF_BOUNDS','ERR_OUT_OF_RANGE','browser_cdp_timeout','browser_cdp_command_failed','browser_cdp_disconnected','NATIVE_PROTECTION_CHANGED','NATIVE_VALUE_ERROR','NATIVE_HELPER_TIMEOUT','NATIVE_DEPENDENCY_MISSING','NATIVE_FILE_MISSING','NATIVE_OS_ERROR','NATIVE_OUTPUT_LIMIT','browser_cdp_socket_error','browser_target_not_found','browser_connection_changed','browser_cdp_response_too_large','browser_action_cancelled','browser_namespace_permission_denied','browser_policy_mount_denied','browser_sandbox_unavailable','browser_display_unavailable','browser_executable_missing','browser_executable_permission_denied','browser_launch_failed','browser_navigation_failed','browser_navigation_identity_missing','browser_action_timeout','browser_action_failed','browser_action_invalid','browser_wait_timeout','browser_wait_invalid','stale_element_reference','stale_document_reference','ENOSPC','EMFILE','ENOMEM']);
// MP-11: closed public codes declared by the shipped controller adapters; no caller-defined code passthrough.
for(const code of 'browser_action_cancelled browser_action_failed browser_action_invalid browser_action_timeout browser_artifact_invalid browser_artifact_unavailable browser_attach_failed browser_cdp_command_failed browser_cdp_disconnected browser_cdp_socket_error browser_cdp_timeout browser_connection_changed browser_controller_internal browser_cookie_writer_fence_busy browser_cookie_writer_fence_timeout browser_debugger_endpoint_unsafe browser_debugger_invalid browser_debugger_unavailable browser_dialog_invalid browser_document_identity_missing browser_download_invalid browser_download_low_disk browser_download_not_active browser_download_unavailable browser_download_unconfigured browser_event_capacity_invalid browser_event_cursor_invalid browser_event_limit_invalid browser_history_action_invalid browser_history_timeout browser_history_unavailable browser_navigation_failed browser_navigation_identity_missing browser_navigation_invalid browser_observation_redacted browser_permission_invalid browser_permission_origin_denied browser_secret_input_exception browser_secret_target_changed browser_secret_target_not_focusable browser_secret_target_not_masked browser_snapshot_invalid browser_submit_failed browser_tab_action_invalid browser_tab_close_failed browser_target_not_found browser_unavailable browser_upload_denied browser_upload_invalid browser_upload_staging_unavailable browser_viewport_invalid browser_wait_invalid browser_wait_timeout invalid_request navigated resync snapshot_frame_limit stale_browser_generation stale_document_reference stale_element_reference viewport_apply_failed'.split(' '))codes.add(code);
// MP-08/MP-10/MP-11: host-only UserDomainRefusal already admits exactly these policy reasons.
for(const reason of ['not_focused_agent','foreign_owner','stale_epoch','stale_reference','not_granted','sensitive_requires_focus'])codes.add('user_domain_'+reason);
for(const code of ['note_shadow_root_limit','note_text_limit'])codes.add(code);
for(const suffix of ['type_error','reference_error','range_error','syntax_error','dom_exception','exception','result_unavailable'])codes.add('note_observer_'+suffix);
const methods=new Set(['Page.captureScreenshot','Page.getFrameTree','Page.getLayoutMetrics','Runtime.evaluate','Runtime.callFunctionOn','DOM.resolveNode','CSS.enable','DOMSnapshot.captureSnapshot','Page.navigate','Page.createIsolatedWorld','Target.activateTarget','Target.attachToTarget','Browser.getVersion','Input.dispatchKeyEvent','Input.dispatchMouseEvent','Input.insertText','Emulation.setFocusEmulationEnabled','Emulation.setDeviceMetricsOverride','Emulation.clearDeviceMetricsOverride','Target.getTargetInfo','Target.getTargets','Page.enable','Page.bringToFront','Runtime.enable','Runtime.releaseObject','DOM.enable','DOM.getDocument','CSS.disable','Page.addScriptToEvaluateOnNewDocument','Page.setLifecycleEventsEnabled','Page.reload']);
const navigationReasons=new Set(['ERR_NAME_NOT_RESOLVED','ERR_CONNECTION_REFUSED','ERR_CONNECTION_TIMED_OUT','ERR_TIMED_OUT','ERR_CERT_COMMON_NAME_INVALID','ERR_CERT_DATE_INVALID','ERR_CERT_AUTHORITY_INVALID','ERR_ABORTED','ERR_INTERNET_DISCONNECTED','ERR_NETWORK_CHANGED','ERR_BLOCKED_BY_ADMINISTRATOR','ERR_BLOCKED_BY_CLIENT','ERR_INVALID_URL']);
const cdpReasons=new Map([['Target closed','target_closed'],['Cannot find context with specified id','execution_context_retired'],['Cannot find execution context with given id','execution_context_retired'],['Could not find node with given id','node_retired'],['No node with given id found','node_retired'],['Session with given id not found','cdp_session_retired']]);
// MP-11: classify only our fixed isolated-observer refusals; never print a
// CDP exception description, page exception, input value or private path.
const observerReasons=new Set(['protected_input_ancestor','stale_node','frame_unavailable','transformed_frame','element_required','invisible_node','occluded_node','offscreen_node','changed_live_target','unavailable_native_text_focus','focus_redirected','control_unavailable','protected_control','control_state_refused','invalid_selection','selection_intersects_protected_content','frame_required','observer_exception']);
export function mirrorExceptionReason(description) {
 const first=typeof description==='string'?description.split('\n',1)[0]:'';
 const reason=/^Error: mirror2 ([a-z ]+)$/.exec(first)?.[1].replaceAll(' ','_');
 return observerReasons.has(reason)?reason:'observer_exception';
}
const fixedReasons=new Map([
 ['MP-11: native region lease retired','native_region_lease_retired'],
 ['MP-11: region lease retired','native_region_lease_retired'],
 ['MP-11: native region attestation geometry','native_region_attestation_geometry'],
 ['MP-11: native region attestation masked','native_region_attestation_masked'],
 ['MP-11: native region attestation mismatch','native_region_attestation_mismatch'],
 ['MP-08: native region readback unavailable','native_region_readback_unavailable'],
 ['MP-11: invalid native window region','native_region_invalid'],
 ['MP-11: invalid native auxiliary region','native_auxiliary_region_invalid'],
 ['MP-11: native region crosses surface boundary','native_region_boundary_refused'],
 ['MP-11: admitted region lease required','native_region_lease_required'],
 ['MP-11: native window inventory lease','native_window_inventory_retired'],
 ['MP-11: native keys held; release desktop input first','native_keys_held'],
 ['MP-08: physical key channel unavailable (Pillow missing)','native_keyboard_pillow_missing'],
 ['MP-08: physical key channel unavailable (Xlib missing)','native_keyboard_xlib_missing'],
 ['MP-08: physical key channel unavailable','native_keyboard_unavailable'],
 ['MP-08: physical key readiness timeout','native_keyboard_readiness_timeout'],
 ['MP-08: physical key channel refused input','native_keyboard_input_refused'],
 ['MP-08: physical key channel ended','native_keyboard_channel_ended'],
 ['MP-08: physical key channel busy','native_keyboard_busy'],

 ['MP-08: desktop readiness timeout','desktop_readiness_timeout'],
 ['MP-08: desktop child exited before readiness','desktop_child_exited_before_readiness'],
 ['MP-08: desktop executable unavailable','desktop_executable_unavailable'],
 ['MP-08: owned desktop unavailable; explicitly stop/start','desktop_unavailable'],
 ['MP-08: AT-SPI bus configuration unavailable','desktop_at_spi_configuration_unavailable'],
 ['MP-08: owned display keymap unavailable','desktop_keymap_unavailable'],
 ['MD-N2: shadow root limit reached','note_shadow_root_limit'],
 ['MD-N2: page text limit reached','note_text_limit'],
 ['MD-N2: observed target/document required','note_reference_required'],
 ['MD-N2: invalid quote','note_quote_invalid'],
 ['MD-N2: stale document; refresh tab','note_document_stale'],
 ['MD-N2: isolated world unavailable','note_world_unavailable'],
 ['MD-N2: isolated observer unavailable','note_observer_unavailable'],
 ['MD-N2: selection observation failed','note_selection_unavailable'],
 ['MD-N2: frame changed during capture','note_frame_changed'],
 ['MD-N2: document changed during capture','note_document_changed'],
 ['MD-N2: isolated frame geometry unavailable','note_frame_geometry_unavailable'],
 ['MD-N2: frame geometry unavailable','note_frame_geometry_unavailable'],
 ['MP-11: stale or foreign mirror','mirror_subscription_retired'],
 ['MP-11: stale mirror document','mirror_document_stale'],
 ['MP-11: stale mirror protection policy or subscription','mirror_policy_stale'],
 ['MP-11: stale mirror protection policy','mirror_policy_stale'],
 ['MP-11: mirror isolated world unavailable','mirror_world_unavailable'],
 ['MP-11: mirror observer unavailable','mirror_observer_unavailable'],
 ['MP-11: frame observer unavailable','mirror_frame_observer_unavailable'],
 ['MP-11: mirror observation unavailable','mirror_observation_unavailable'],
 ['MP-11: closed-root frame snapshot unavailable','mirror_frame_snapshot_unavailable'],
 ['MD-2: install native Chromium or set CHARIOX_KERNEL_BROWSER_EXECUTABLE','browser_executable_unavailable'],
 ['MD-2: run the kernel browser as a normal Unix user; root Chromium is unsupported','browser_root_user_unsupported'],
 ['MD-2: browser profile is already owned by a live process','browser_profile_in_use'],
 ['MD-2: host tab limit reached','browser_tab_limit'],
 ['MD-2: frame exceeds limit','browser_frame_limit'],
 ['MD-2: frame subscription limit reached','browser_subscription_limit'],
 ['MD-2: browser keepalive window unavailable','browser_keepalive_unavailable'],
 ['MD-2: browser tab registry is unreadable; preserve it for recovery','browser_registry_unreadable'],
 ['MD-2: invalid browser tab registry','browser_registry_invalid'],
 ['MD-2: unsupported browser operation','browser_operation_unsupported'],
 ['MD-2: unsupported host method','browser_method_unsupported'],
 ['MD-2: sandboxed host Chromium did not become ready (check executable, display and profile ownership)','browser_launch_failed'],
 ['MP-11: changed mirror input target','input_target_changed'],
 ['MP-11: stale mirror input document','input_document_stale'],
 ['MP-11: stale mirror input epoch','input_epoch_stale'],
 ['MP-11: stale mirror protection policy or admitted input','input_policy_stale'],
 ['MP-11: unavailable native text focus','input_focus_unavailable'],
 ["MP-11: extensions are the person's own; an agent cannot open the Web Store or chrome://extensions or act on them",'extension_management_not_granted'],
]);
for(const name of ['bwrap','Xvfb','dbus-daemon','openbox','setxkbmap'])fixedReasons.set(`MP-08: owned desktop prerequisite unavailable (${name})`,'desktop_prerequisite_'+name.toLowerCase().replaceAll('-','_')+'_unavailable');
// Worker stages come from the shipped native executable; admit exact public labels only.
const nativeStages=new Set('native_codec_unavailable x_extensions desktop_geometry window_tree_bound window_ambiguous window_not_found window_retired composite_redirect composite_pixmap native_admission native_pool_metadata native_pool_owner native_packet_root native_root_owner native_capture_unavailable native_window_retired native_config native_config_pipe native_config_bound'.split(' '));
// MIRROR-ROBUST-STALE-REF: navigation and teardown races the viewer answers by rebinding are not host errors;
// a binding that is really stale shows as the viewer failing to follow, never as one of these.
const races=new Set(['region_retired','display_suspended','display_retired']);
export function hostErrorLabel(error) {
 if(races.has(error?.race))return 'navigation_race '+error.race;
 if(error?.code==='user_domain_stale_reference'||error?.code==='user_domain_stale_epoch')return 'navigation_race '+error.code;
 if(error?.code==='stale_document_reference')return 'navigation_race stale_document';
 const name=names.has(error?.name)?error.name:'Error';
 const known=typeof error?.stack==='string'?[...error.stack.matchAll(/\/([a-z0-9-]+\.mjs):(\d{1,5}):(\d{1,5})/g)].filter(m=>files.has(m[1])):[];
 // A timeout fires from a timer inside the CDP client: the waiting caller is the first position after it.
 const callers=error?.code==='browser_cdp_timeout'?known.filter(m=>m[1]!=='browser-controller-cdp.mjs').slice(0,3):[];
 const frames=callers.length?callers:known.slice(0,1),frame=frames[0];
 const description=typeof error?.message==='string'?error.message:'';
 const command=error?.code==='browser_cdp_command_failed'?/^([A-Za-z.]+): (.*)$/.exec(description):null;
 const method=error?.code==='browser_cdp_timeout'?/^browser CDP method ([A-Za-z.]+) timed out after [0-9]+ms$/.exec(description)?.[1]:command?.[1];
 const desktopExit=/^MP-08: desktop failed to become ready \((bwrap|Xvfb|dbus-daemon|openbox), exit ([0-9]{1,3}|signal)\)$/.exec(description);
 const navigation=error?.code==='browser_navigation_failed'?/^browser navigation failed: net::([A-Z_]+)$/.exec(description)?.[1]:null;
 return 'host_error '+name+(codes.has(error?.code)?' '+error.code:'')+(method&&methods.has(method)?' '+method:'')+(command&&cdpReasons.has(command[2])?' '+cdpReasons.get(command[2]):'')+(navigation&&navigationReasons.has(navigation)?' net::'+navigation:'')+(observerReasons.has(error?.mirrorReason)?' '+error.mirrorReason:'')+(desktopExit?' desktop_'+desktopExit[1].toLowerCase().replaceAll('-','_')+'_failed_to_start':'')+(codes.has(error?.code)&&/^note_observer_/.test(error.code)&&Number.isSafeInteger(error?.noteObserverLine)&&error.noteObserverLine>0&&error.noteObserverLine<=256?' note_observer:'+error.noteObserverLine:'')+(nativeStages.has(error?.nativeStage)?' native_worker_'+error.nativeStage:'')+(fixedReasons.has(error?.message)?' '+fixedReasons.get(error.message):'')+(frame?' '+frames.map(m=>m[1]+':'+m[2]+':'+m[3]).join(' < '):'');
}

// MP-08/MP-10/MP-11: shared public reply and log classifier. Exception messages,
// CDP descriptions, arguments and private path prefixes never cross this boundary.
// Keep kernel_browser_failed as the outer code; no serialized contract changes.
// Existing Web clients match this sentence when retrying a read; append the cause.
export function hostFailureMessage(error) {
 const label=hostErrorLabel(error);
 return 'MD-2: host browser operation failed; refresh state or check host browser readiness ('+(label.startsWith('host_error ')?label.slice('host_error '.length):label)+')';
}

// MP-08/MP-10/MP-11: classify bounded startup stderr; never expose its text,
// executable/profile paths, URLs or environment. Unknown output stays generic.
export function browserLaunchFailureReason(stderr, spawnCode) {
 if(spawnCode==='ENOENT')return 'browser_executable_missing';
 if(spawnCode==='EACCES')return 'browser_executable_permission_denied';
 const text=typeof stderr==='string'?stderr.slice(0,8192):'';
 if(/unshare: (?:write failed \/proc\/self\/(?:uid|gid)_map: Operation not permitted|unshare failed: Operation not permitted)/.test(text))return 'browser_namespace_permission_denied';
 if(/mount: \/etc: (?:permission denied|must be superuser)/.test(text))return 'browser_policy_mount_denied';
 if(/No usable sandbox|Failed to move to new namespace.*Operation not permitted/.test(text))return 'browser_sandbox_unavailable';
 if(/Missing X server or \$DISPLAY|cannot open display|Unable to open X display/.test(text))return 'browser_display_unavailable';
 return 'browser_launch_failed';
}

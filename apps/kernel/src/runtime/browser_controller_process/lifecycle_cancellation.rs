use super::*;
use crate::transport::room_browser_controller::{
    BrowserLifecycleOperation as Operation, RoomBrowserControllerResult as Response,
};
use sha2::{Digest, Sha256};

impl BrowserControllerProcessStore {
    pub(crate) fn perform_cancellable_browser_lifecycle(
        &self,
        room: &str,
        execution_id: &str,
        target: &str,
        document: &str,
        operation: &Operation,
    ) -> Result<Response, String> {
        let unavailable = match operation {
            Operation::Tab { .. } => Response::Tab { result: None },
            Operation::History { .. } => Response::History { result: None },
            Operation::Navigate { .. } => Response::Navigation { result: None },
            Operation::Dialog { .. } => Response::Dialog { result: None },
        };
        let (method, params) = match operation {
            Operation::Tab { action } => (
                "browser.tab",
                serde_json::json!({
                    "target_id": target,
                    "document_id": document,
                    "action": action.as_str(),
                }),
            ),
            Operation::History { action } => (
                "browser.history",
                serde_json::json!({
                    "target_id": target,
                    "document_id": document,
                    "action": action.as_str(),
                }),
            ),
            Operation::Navigate { url } => {
                let url = normalize_browser_navigation_url(url.as_str())?;
                (
                    "browser.navigate",
                    serde_json::json!({
                        "target_id": target,
                        "document_id": document,
                        "url": url,
                    }),
                )
            }
            Operation::Dialog { action } => {
                action.validate()?;
                (
                    "browser.dialog",
                    serde_json::json!({
                        "target_id": target,
                        "document_id": document,
                        "action": action.kind(),
                        "prompt_text": action.prompt_text(),
                    }),
                )
            }
        };
        self.perform_cancellable_tab_mutation(
            room,
            execution_id,
            fingerprint(target, document, operation)?,
            unavailable,
            target,
            method,
            params,
            move |response| match operation {
                Operation::Tab { action } => {
                    let result =
                        response.into_result::<BrowserControllerTabResult>("browser.tab")?;
                    result.validate(target, document, *action)?;
                    Ok(Response::Tab {
                        result: Some(result),
                    })
                }
                Operation::History { action } => {
                    let result = response
                        .into_result::<BrowserControllerHistoryResult>("browser.history")?;
                    result.validate(target, *action)?;
                    Ok(Response::History {
                        result: Some(result),
                    })
                }
                Operation::Navigate { url } => {
                    let expected_url = normalize_browser_navigation_url(url.as_str())?;
                    let result = response
                        .into_result::<BrowserControllerNavigationResult>("browser.navigate")?;
                    result.validate(target, &expected_url)?;
                    Ok(Response::Navigation {
                        result: Some(result),
                    })
                }
                Operation::Dialog { action } => {
                    let result =
                        response.into_result::<BrowserControllerDialogResult>("browser.dialog")?;
                    result.validate(target, document, action)?;
                    Ok(Response::Dialog {
                        result: Some(result),
                    })
                }
            },
        )
    }

    pub(crate) fn recover_cancellable_browser_lifecycle(
        &self,
        room: &str,
        execution_id: &str,
        target: &str,
        document: &str,
        operation: &Operation,
    ) -> Result<Response, String> {
        self.recover_cancellable_operation(
            room,
            execution_id,
            fingerprint(target, document, operation)?,
        )
    }
}

fn fingerprint(target: &str, document: &str, operation: &Operation) -> Result<[u8; 32], String> {
    let request = serde_json::to_vec(&("lifecycle", target, document, operation))
        .map_err(|_| "failed to fingerprint browser lifecycle operation")?;
    Ok(Sha256::digest(request).into())
}

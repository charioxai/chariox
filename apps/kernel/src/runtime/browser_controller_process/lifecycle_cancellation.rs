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
        self.perform_cancellable_operation(
            room,
            execution_id,
            fingerprint(target, document, operation)?,
            unavailable,
            |backend| match operation {
                Operation::Tab { action } => backend
                    .manage_browser_tab(target, document, *action)
                    .map(|result| Response::Tab {
                        result: Some(result),
                    }),
                Operation::History { action } => backend
                    .navigate_browser_history(target, document, *action)
                    .map(|result| Response::History {
                        result: Some(result),
                    }),
                Operation::Navigate { url } => backend
                    .navigate_browser(target, document, url.as_str())
                    .map(|result| Response::Navigation {
                        result: Some(result),
                    }),
                Operation::Dialog { action } => backend
                    .handle_browser_dialog(target, document, action)
                    .map(|result| Response::Dialog {
                        result: Some(result),
                    }),
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

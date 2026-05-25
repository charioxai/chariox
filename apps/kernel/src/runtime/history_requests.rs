//! History request use cases.

mod prompt_input;
mod query;
mod semantic;
mod session;

pub(crate) use prompt_input::{
    execute_prompt_input_history_request, execute_record_prompt_input_history_request,
};
pub(crate) use query::{
    execute_query_recall_request, recall_query_from_request, recall_query_from_search_request,
};
pub(crate) use semantic::{
    knn_semantic_recall_search, semantic_recall_request_from_utility_input,
    semantic_recall_utility_input_from_search_request,
};
pub(crate) use session::execute_session_history_request_from_session;

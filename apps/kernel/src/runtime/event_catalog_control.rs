use std::collections::BTreeMap;
use std::io::Read;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use crate::error::DaemonError;
use crate::local::{
    EventCatalogCategory, EventCatalogFacet, EventCatalogFacetValue, EventGeneratorCatalogDetail,
    EventGeneratorCatalogPage, EventGeneratorCatalogSummary, EventGeneratorEventDefinition,
    EventGeneratorEventPage, EventGeneratorParty, LocalDaemonRequest, LocalDaemonResponse,
};
use crate::runtime::projection::DaemonConfigProjectionStore;
use crate::runtime::state::KernelRuntimeState;

const CATALOG_CACHE_FRESH_TTL: Duration = Duration::from_secs(60);
const CATALOG_CACHE_STALE_TTL: Duration = Duration::from_secs(5 * 60);
const CATALOG_CACHE_MAX_ENTRIES: usize = 128;
const CATALOG_RESPONSE_MAX_BYTES: u64 = 2 * 1024 * 1024;

#[derive(Clone)]
struct CatalogCacheEntry {
    response: LocalDaemonResponse,
    stored_at: Instant,
}

static CATALOG_CACHE: OnceLock<Mutex<BTreeMap<String, CatalogCacheEntry>>> = OnceLock::new();

pub(crate) async fn execute_event_catalog_request(
    runtime_state: &KernelRuntimeState,
    config_projection: &DaemonConfigProjectionStore,
    request: LocalDaemonRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let config = config_projection.snapshot();
    if matches!(request, LocalDaemonRequest::GetEventDeliveryStatus(_)) {
        return Ok(LocalDaemonResponse::EventDeliveryStatus {
            status: crate::transport::event_delivery_client::event_delivery_status(
                runtime_state,
                &config,
            ),
        });
    }
    if matches!(
        request,
        LocalDaemonRequest::StartEventGeneratorAuthorization(_)
            | LocalDaemonRequest::ListEventGeneratorResources(_)
    ) {
        let targets = config.event_generator_management_targets.clone();
        return tokio::task::spawn_blocking(move || aegs_management_request(&targets, &request))
            .await
            .map_err(|error| DaemonError::LocalTransport {
                operation: "query event generator management service",
                message: error.to_string(),
            })?;
    }
    let registry_url = config.event_registry_url.clone();
    tokio::task::spawn_blocking(move || {
        if let Some(registry_url) = registry_url {
            cached_remote_catalog_request(&registry_url, &request)
        } else {
            builtin_catalog_request(&request)
        }
    })
    .await
    .map_err(|error| DaemonError::LocalTransport {
        operation: "query event generator catalog",
        message: error.to_string(),
    })?
}

fn aegs_management_request(
    targets: &BTreeMap<String, crate::config::EventGeneratorManagementTarget>,
    request: &LocalDaemonRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let (generator_id, path, body) = match request {
        LocalDaemonRequest::StartEventGeneratorAuthorization(request) => (
            request.generator_id.as_str(),
            "/v1/authorizations",
            serde_json::to_string(&arroba_event_protocol::AegsAuthorizationStartRequest {
                generator_id: request.generator_id.clone(),
                return_url: request.return_url.clone(),
            })
            .map_err(|error| catalog_error(error.to_string()))?,
        ),
        LocalDaemonRequest::ListEventGeneratorResources(request) => (
            request.generator_id.as_str(),
            "/v1/resources/query",
            serde_json::to_string(&arroba_event_protocol::AegsProviderResourceQuery {
                generator_id: request.generator_id.clone(),
                connection_id: request.connection_id.clone(),
                query: request.query.clone(),
                cursor: request.cursor.clone(),
                limit: request.limit,
            })
            .map_err(|error| catalog_error(error.to_string()))?,
        ),
        _ => {
            return Err(catalog_error(
                "request is not an event generator management request".to_string(),
            ))
        }
    };
    let target = targets.get(generator_id).ok_or_else(|| {
        catalog_error(format!(
            "event generator `{generator_id}` has no configured management target"
        ))
    })?;
    let url = format!("{}{path}", target.url);
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(3))
        .timeout_read(Duration::from_secs(10))
        .timeout_write(Duration::from_secs(10))
        .build();
    let response = agent
        .post(&url)
        .set("authorization", &format!("Bearer {}", target.token))
        .set("content-type", "application/json")
        .send_string(&body)
        .map_err(|error| catalog_error(format!("AEGS {generator_id} request failed: {error}")))?;
    let response = response
        .into_string()
        .map_err(|error| catalog_error(error.to_string()))?;
    match request {
        LocalDaemonRequest::StartEventGeneratorAuthorization(_) => {
            let flow = serde_json::from_str(&response)
                .map_err(|error| catalog_error(format!("AEGS response is invalid: {error}")))?;
            Ok(LocalDaemonResponse::EventGeneratorAuthorizationStarted { flow })
        }
        LocalDaemonRequest::ListEventGeneratorResources(_) => {
            let page = serde_json::from_str(&response)
                .map_err(|error| catalog_error(format!("AEGS response is invalid: {error}")))?;
            Ok(LocalDaemonResponse::EventGeneratorResourcesPage { page })
        }
        _ => unreachable!("event generator management request was matched above"),
    }
}

fn cached_remote_catalog_request(
    registry_url: &str,
    request: &LocalDaemonRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let key = format!(
        "{}|protocol={}|{}",
        registry_url,
        arroba_event_protocol::EVENT_DELIVERY_PROTOCOL_VERSION,
        serde_json::to_string(request).map_err(|error| catalog_error(error.to_string()))?
    );
    let cache = CATALOG_CACHE.get_or_init(|| Mutex::new(BTreeMap::new()));
    if let Some(response) = cache
        .lock()
        .ok()
        .and_then(|cache| cache.get(&key).cloned())
        .filter(|entry| entry.stored_at.elapsed() <= CATALOG_CACHE_FRESH_TTL)
        .map(|entry| entry.response)
    {
        return Ok(response);
    }
    match remote_catalog_request(registry_url, request) {
        Ok(response) => {
            if let Ok(mut cache) = cache.lock() {
                if cache.len() >= CATALOG_CACHE_MAX_ENTRIES {
                    if let Some(oldest) = cache
                        .iter()
                        .min_by_key(|(_, entry)| entry.stored_at)
                        .map(|(key, _)| key.clone())
                    {
                        cache.remove(&oldest);
                    }
                }
                cache.insert(
                    key,
                    CatalogCacheEntry {
                        response: response.clone(),
                        stored_at: Instant::now(),
                    },
                );
            }
            Ok(response)
        }
        Err(error) => {
            let cached = cache
                .lock()
                .ok()
                .and_then(|cache| cache.get(&key).cloned())
                .filter(|entry| entry.stored_at.elapsed() <= CATALOG_CACHE_STALE_TTL);
            cached
                .map(|entry| mark_catalog_response_stale(entry.response))
                .ok_or(error)
        }
    }
}

fn mark_catalog_response_stale(response: LocalDaemonResponse) -> LocalDaemonResponse {
    match response {
        LocalDaemonResponse::EventGeneratorCatalogPage { mut page } => {
            page.stale = true;
            LocalDaemonResponse::EventGeneratorCatalogPage { page }
        }
        response => response,
    }
}

fn remote_catalog_request(
    registry_url: &str,
    request: &LocalDaemonRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let mut url =
        url::Url::parse(registry_url).map_err(|error| catalog_error(error.to_string()))?;
    match request {
        LocalDaemonRequest::GetEventGeneratorCatalogLanding(request) => {
            url.set_path("/v1/event-generators");
            url.query_pairs_mut()
                .append_pair("view", "landing")
                .append_pair("limit", &bounded_limit(request.limit).to_string());
            fetch_page(url)
        }
        LocalDaemonRequest::SearchEventGeneratorCatalog(request) => {
            url.set_path("/v1/event-generators");
            let mut query = url.query_pairs_mut();
            query
                .append_pair("q", request.query.trim())
                .append_pair("limit", &bounded_limit(request.limit).to_string());
            if let Some(category) = request.category.as_deref() {
                query.append_pair("category", category);
            }
            if let Some(verification) = request.verification.as_deref() {
                query.append_pair("verification", verification);
            }
            if let Some(cursor) = request.cursor.as_deref() {
                query.append_pair("cursor", cursor);
            }
            drop(query);
            fetch_page(url)
        }
        LocalDaemonRequest::BrowseEventGeneratorCategory(request) => {
            url.set_path("/v1/event-generators");
            let mut query = url.query_pairs_mut();
            query
                .append_pair("category", request.category.trim())
                .append_pair("limit", &bounded_limit(request.limit).to_string());
            if let Some(cursor) = request.cursor.as_deref() {
                query.append_pair("cursor", cursor);
            }
            drop(query);
            fetch_page(url)
        }
        LocalDaemonRequest::GetEventGeneratorDetail(request) => {
            url.set_path(&format!(
                "/v1/event-generators/{}",
                percent_encode_path_segment(&request.generator_id)
            ));
            if let Some(version) = request.version.as_deref() {
                url.query_pairs_mut().append_pair("version", version);
            }
            let detail = fetch_json::<EventGeneratorCatalogDetail>(url)?;
            Ok(LocalDaemonResponse::EventGeneratorDetail { detail })
        }
        LocalDaemonRequest::BrowseEventGeneratorEvents(request) => {
            url.set_path(&format!(
                "/v1/event-generators/{}/events",
                percent_encode_path_segment(&request.generator_id)
            ));
            let mut query = url.query_pairs_mut();
            query.append_pair("limit", &bounded_event_limit(request.limit).to_string());
            if let Some(search) = request.query.as_deref() {
                query.append_pair("query", search);
            }
            if let Some(cursor) = request.cursor.as_deref() {
                query.append_pair("cursor", cursor);
            }
            drop(query);
            let page = fetch_json::<EventGeneratorEventPage>(url)?;
            Ok(LocalDaemonResponse::EventGeneratorEventsPage { page })
        }
        _ => Err(catalog_error(
            "request is not an event catalog request".to_string(),
        )),
    }
}

fn builtin_catalog_request(
    request: &LocalDaemonRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    match request {
        LocalDaemonRequest::GetEventGeneratorCatalogLanding(request) => {
            Ok(LocalDaemonResponse::EventGeneratorCatalogPage {
                page: builtin_page("", None, None, 0, bounded_limit(request.limit)),
            })
        }
        LocalDaemonRequest::SearchEventGeneratorCatalog(request) => {
            let cursor = decode_cursor(request.cursor.as_deref())?;
            Ok(LocalDaemonResponse::EventGeneratorCatalogPage {
                page: builtin_page(
                    &request.query,
                    request.category.as_deref(),
                    request.verification.as_deref(),
                    cursor,
                    bounded_limit(request.limit),
                ),
            })
        }
        LocalDaemonRequest::BrowseEventGeneratorCategory(request) => {
            let cursor = decode_cursor(request.cursor.as_deref())?;
            Ok(LocalDaemonResponse::EventGeneratorCatalogPage {
                page: builtin_page(
                    "",
                    Some(&request.category),
                    None,
                    cursor,
                    bounded_limit(request.limit),
                ),
            })
        }
        LocalDaemonRequest::GetEventGeneratorDetail(request) => {
            let detail = builtin_detail(&request.generator_id).ok_or_else(|| {
                catalog_error(format!(
                    "event generator `{}` was not found",
                    request.generator_id
                ))
            })?;
            Ok(LocalDaemonResponse::EventGeneratorDetail { detail })
        }
        LocalDaemonRequest::BrowseEventGeneratorEvents(request) => {
            let detail = builtin_detail(&request.generator_id).ok_or_else(|| {
                catalog_error(format!(
                    "event generator `{}` was not found",
                    request.generator_id
                ))
            })?;
            let search = request
                .query
                .as_deref()
                .unwrap_or_default()
                .trim()
                .to_ascii_lowercase();
            let offset = decode_cursor(request.cursor.as_deref())?;
            let events = detail
                .events
                .into_iter()
                .filter(|event| {
                    search.is_empty()
                        || event.event_type.to_ascii_lowercase().contains(&search)
                        || event.name.to_ascii_lowercase().contains(&search)
                })
                .collect::<Vec<_>>();
            let end = (offset + bounded_event_limit(request.limit) as usize).min(events.len());
            let page_events = if offset >= events.len() {
                Vec::new()
            } else {
                events[offset..end].to_vec()
            };
            Ok(LocalDaemonResponse::EventGeneratorEventsPage {
                page: EventGeneratorEventPage {
                    events: page_events,
                    next_cursor: (end < events.len()).then(|| format!("offset:{end}")),
                },
            })
        }
        _ => Err(catalog_error(
            "request is not an event catalog request".to_string(),
        )),
    }
}

fn builtin_page(
    query: &str,
    category: Option<&str>,
    verification: Option<&str>,
    offset: usize,
    limit: u32,
) -> EventGeneratorCatalogPage {
    let query = query.trim().to_ascii_lowercase();
    let mut services = builtin_summaries()
        .into_iter()
        .filter(|service| {
            (query.is_empty()
                || [
                    service.name.as_str(),
                    service.summary.as_str(),
                    service.provider.as_str(),
                    service.generator_id.as_str(),
                ]
                .iter()
                .any(|value| value.to_ascii_lowercase().contains(&query)))
                && category.is_none_or(|category| {
                    service
                        .categories
                        .iter()
                        .any(|value| value.eq_ignore_ascii_case(category))
                })
                && verification.is_none_or(|verification| service.verification == verification)
        })
        .collect::<Vec<_>>();
    services.sort_by_key(|service| {
        (
            !service.recommended,
            std::cmp::Reverse(service.installed_count),
        )
    });
    let total = services.len();
    let end = (offset + limit as usize).min(total);
    let services = if offset >= total {
        Vec::new()
    } else {
        services[offset..end].to_vec()
    };
    EventGeneratorCatalogPage {
        services,
        next_cursor: (end < total).then(|| format!("offset:{end}")),
        categories: builtin_categories(),
        facets: vec![EventCatalogFacet {
            id: "verification".to_string(),
            values: vec![EventCatalogFacetValue {
                value: "arroba".to_string(),
                count: 1,
            }],
        }],
        stale: false,
    }
}

fn builtin_summaries() -> Vec<EventGeneratorCatalogSummary> {
    vec![EventGeneratorCatalogSummary {
        schema_version: 1,
        generator_id: "dev.arroba.dummy".to_string(),
        version: "1.0.0".to_string(),
        name: "Dummy Events".to_string(),
        summary: "Deterministic event source for local and deployment validation.".to_string(),
        provider: "Arroba test harness".to_string(),
        publisher: EventGeneratorParty {
            id: "dev.arroba".to_string(),
            name: "Arroba".to_string(),
            url: Some("https://arroba.dev".to_string()),
        },
        operator: EventGeneratorParty {
            id: "local".to_string(),
            name: "Local operator".to_string(),
            url: None,
        },
        verification: "arroba".to_string(),
        manifest_digest: "sha256:03b694a033a58a9e29b8ed947c6f5fb332def989b4deb0f5810f2b6e6bbd054f"
            .to_string(),
        protocol_version: arroba_event_protocol::EVENT_DELIVERY_PROTOCOL_VERSION,
        categories: vec!["Developer tools".to_string(), "Testing".to_string()],
        installed_count: 0,
        recommended: true,
        availability: "available".to_string(),
    }]
}

fn builtin_detail(generator_id: &str) -> Option<EventGeneratorCatalogDetail> {
    let summary = builtin_summaries()
        .into_iter()
        .find(|summary| summary.generator_id == generator_id)?;
    Some(EventGeneratorCatalogDetail {
        summary,
        authorization: serde_json::json!({
            "kind": "none",
            "disclosure": "This deterministic fixture does not contact an external provider."
        }),
        events: vec![EventGeneratorEventDefinition {
            event_type: "dummy.test".to_string(),
            version: 1,
            name: "Test event".to_string(),
            description: "Emits a user-supplied prompt and optional artifact references."
                .to_string(),
            filter_schema: serde_json::json!({
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "channel": {"type": "string", "default": "default"}
                }
            }),
            required_scopes: Vec::new(),
        }],
        signature: serde_json::json!({
            "key_id": "dev.arroba.fixture.2026-07",
            "algorithm": "ed25519",
            "digest": "sha256:03b694a033a58a9e29b8ed947c6f5fb332def989b4deb0f5810f2b6e6bbd054f",
            "value": "E19Yh8KIo5BA+YlDdVeA6mGnwSUxp7CDWCsCiD3VF/WusEWTnfYqNtmCbyWuNsUIl1mqfvC/w0vi4swC579+Ag=="
        }),
        deprecation: None,
    })
}

fn builtin_categories() -> Vec<EventCatalogCategory> {
    [
        ("developer-tools", "Developer tools", 1),
        ("testing", "Testing", 1),
    ]
    .into_iter()
    .map(|(id, name, service_count)| EventCatalogCategory {
        id: id.to_string(),
        name: name.to_string(),
        service_count,
    })
    .collect()
}

fn fetch_page(url: url::Url) -> Result<LocalDaemonResponse, DaemonError> {
    let page = fetch_json::<EventGeneratorCatalogPage>(url)?;
    Ok(LocalDaemonResponse::EventGeneratorCatalogPage { page })
}

fn fetch_json<T: serde::de::DeserializeOwned>(url: url::Url) -> Result<T, DaemonError> {
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(3))
        .timeout_read(Duration::from_secs(5))
        .timeout_write(Duration::from_secs(5))
        .build();
    let response = agent
        .get(url.as_str())
        .set(
            "x-arroba-event-protocol-version",
            &arroba_event_protocol::EVENT_DELIVERY_PROTOCOL_VERSION.to_string(),
        )
        .call()
        .map_err(|error| catalog_error(format!("registry request failed: {error}")))?;
    let mut body = String::new();
    response
        .into_reader()
        .take(CATALOG_RESPONSE_MAX_BYTES + 1)
        .read_to_string(&mut body)
        .map_err(|error| catalog_error(error.to_string()))?;
    if body.len() as u64 > CATALOG_RESPONSE_MAX_BYTES {
        return Err(catalog_error(
            "registry response exceeded 2 MiB".to_string(),
        ));
    }
    serde_json::from_str(&body).map_err(|error| catalog_error(error.to_string()))
}

fn bounded_limit(limit: u32) -> u32 {
    limit.clamp(1, 50)
}

fn bounded_event_limit(limit: u32) -> u32 {
    limit.clamp(1, 100)
}

fn decode_cursor(cursor: Option<&str>) -> Result<usize, DaemonError> {
    let Some(cursor) = cursor else {
        return Ok(0);
    };
    cursor
        .strip_prefix("offset:")
        .and_then(|value| value.parse::<usize>().ok())
        .ok_or_else(|| catalog_error("invalid event catalog cursor".to_string()))
}

fn percent_encode_path_segment(value: &str) -> String {
    url::form_urlencoded::byte_serialize(value.as_bytes()).collect()
}

fn catalog_error(message: String) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "query event generator catalog",
        message,
    }
}

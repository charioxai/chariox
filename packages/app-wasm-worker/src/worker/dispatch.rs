use crate::{
    attribution::Attribution,
    bindings::{
        chariox::app::app_types::{Actor, ActorKind, InvocationContext, Wake},
        Worker,
    },
    host::{json as app_json, remote_error, Host},
    ipc::{error, wall_ms, Ipc},
    limits::Limits,
    scope::Scope,
};
use chariox_app_runtime::wire::{Message, RemoteError};
use futures::{future::BoxFuture, stream::FuturesUnordered, StreamExt};
use serde_json::Value;
use std::{
    collections::{HashMap, VecDeque},
    time::Duration,
};
use tokio::sync::mpsc;
use wasmtime::component::{Accessor, FutureReader};
const MAX_CALLS: usize = 64;
#[derive(Clone)]
pub(super) enum Operation {
    Tool {
        name: String,
        input: String,
    },
    LiveEvent {
        name: String,
        data: String,
    },
    Event {
        name: String,
        occurrence_id: String,
        payload: String,
    },
    Lifecycle {
        event: String,
        data: Option<String>,
    },
    Wake {
        wake: Wake,
        overdue: bool,
    },
}
#[derive(Clone)]
pub(super) struct Request {
    pub(super) operation: Operation,
    pub(super) context: InvocationContext,
    pub(super) deadline: tokio::time::Instant,
}
pub(super) struct Active {
    pub(super) request: Request,
    pub(super) scope: Scope,
    pub(super) attempts: u8,
    pub(super) cancellation: Option<&'static str>,
}
impl Drop for Active {
    fn drop(&mut self) {
        self.scope.finish();
    }
}
struct Completed {
    id: String,
    result: Result<Value, RemoteError>,
    force: Option<&'static str>,
}
fn invocation(deadline_ms: u64, value: Option<&Value>) -> Result<InvocationContext, RemoteError> {
    let field = |key: &str| {
        value
            .and_then(|value| value[key].as_str())
            .map(String::from)
    };
    let actor = match value.and_then(|value| value.get("actor")) {
        None | Some(Value::Null) => None,
        Some(value) => Some(Actor {
            kind: match value["kind"].as_str() {
                Some("human") => ActorKind::Human,
                Some("agent") => ActorKind::Agent,
                Some("background") => ActorKind::Background,
                _ => return Err(error("INVALID_ARGUMENT", "invalid invocation actor")),
            },
            id: value["id"]
                .as_str()
                .ok_or_else(|| error("INVALID_ARGUMENT", "missing actor id"))?
                .into(),
        }),
    };
    Ok(InvocationContext {
        deadline_ms,
        installation_id: field("installation_id"),
        room_id: field("room_id"),
        operation_id: field("operation_id"),
        agent_id: field("agent_id"),
        task_id: field("task_id"),
        turn_id: field("turn_id"),
        actor,
    })
}
impl Request {
    fn parse(
        method: &str,
        params: Value,
        deadline_ms: u64,
        context: Option<Value>,
        limits: Limits,
    ) -> Result<Self, RemoteError> {
        let text = |key: &str| {
            params[key]
                .as_str()
                .map(String::from)
                .ok_or_else(|| error("INVALID_ARGUMENT", format!("missing {key}")))
        };
        let operation = match method {
            "tools.invoke" => Operation::Tool {
                name: text("name")?,
                input: params["input"].to_string(),
            },
            "events.deliver"
                if params.get("data").is_some() && params.get("occurrence_id").is_none() =>
            {
                Operation::LiveEvent {
                    name: text("name")?,
                    data: params["data"].to_string(),
                }
            }
            "events.deliver" => Operation::Event {
                name: text("name")?,
                occurrence_id: text("occurrence_id")?,
                payload: params["payload"].to_string(),
            },
            "lifecycle.dispatch" => Operation::Lifecycle {
                event: text("event")?,
                data: params.get("data").map(Value::to_string),
            },
            "schedule.wake" => Operation::Wake {
                wake: Wake {
                    id: text("id")?,
                    due_at_ms: params["dueAtMs"]
                        .as_u64()
                        .ok_or_else(|| error("INVALID_ARGUMENT", "missing dueAtMs"))?,
                    revision: params["revision"].as_str().map(String::from),
                },
                overdue: params["overdue"]
                    .as_bool()
                    .ok_or_else(|| error("INVALID_ARGUMENT", "missing overdue"))?,
            },
            _ => return Err(error("METHOD_NOT_FOUND", "unknown App handler")),
        };
        let remaining = deadline_ms.saturating_sub(wall_ms()).min(limits.call_ms);
        if remaining == 0 {
            return Err(error(
                "DEADLINE_EXCEEDED",
                "App call deadline already expired",
            ));
        }
        Ok(Self {
            operation,
            context: invocation(deadline_ms, context.as_ref())?,
            deadline: tokio::time::Instant::now() + Duration::from_millis(remaining),
        })
    }
}
async fn invoke(
    accessor: &Accessor<Host>,
    worker: &Worker,
    request: Request,
    scope: Scope,
    limits: Limits,
    attribution: &Attribution,
) -> Completed {
    let id = scope.id().to_string();
    if let Some(reason) = scope.reason() {
        return Completed {
            id,
            result: Err(error(reason, "App call cancelled before dispatch")),
            force: None,
        };
    }
    if let Operation::LiveEvent { ref name, ref data } = request.operation {
        let subscriptions = accessor.with(|mut access| access.get().subscriptions.clone());
        let result = tokio::select! {
            reason=scope.cancelled()=>Err(error(reason,"live event cancelled")),
            _=tokio::time::sleep_until(scope.deadline())=>Err(error("DEADLINE_EXCEEDED","live event deadline exceeded")),
            result=subscriptions.deliver(name,data.clone())=>result.map(|()|Value::Null).map_err(remote_error),
        };
        return Completed {
            id,
            result,
            force: None,
        };
    }
    let signal = scope.clone();
    let cancel = match accessor.with(|mut access| {
        FutureReader::new(&mut access, async move {
            Ok::<_, wasmtime::Error>(signal.guest_cancelled().await.into())
        })
    }) {
        Ok(value) => value,
        Err(_) => {
            return Completed {
                id,
                result: Err(error("APP_RESOURCE_LIMIT", "future table full")),
                force: None,
            }
        }
    };
    attribution.starting(scope.clone());
    let call = async {
        let handlers = worker.chariox_app_handlers();
        let result = match request.operation {
            Operation::LiveEvent { .. } => unreachable!("live event already dispatched"),
            Operation::Tool { name, input } => handlers
                .call_call_tool(accessor, name, input, request.context, cancel)
                .await?
                .map(Some),
            Operation::Event {
                name,
                occurrence_id,
                payload,
            } => handlers
                .call_deliver_event(
                    accessor,
                    name,
                    occurrence_id,
                    payload,
                    request.context,
                    cancel,
                )
                .await?
                .map(Some),
            Operation::Lifecycle { event, data } => {
                handlers
                    .call_lifecycle(accessor, event, data, request.context, cancel)
                    .await?
            }
            Operation::Wake { wake, overdue } => handlers
                .call_deliver_wake(accessor, wake, overdue, request.context, cancel)
                .await?
                .map(|()| None),
        };
        Ok::<_, wasmtime::Error>(match result {
            Ok(Some(value)) => app_json(&value)
                .map_err(|_| error("INVALID_OUTPUT", "App handler returned invalid JSON")),
            Ok(None) => Ok(Value::Null),
            Err(error) => Err(remote_error(error)),
        })
    };
    tokio::pin!(call);
    let ending = async {
        tokio::select! {
            biased;
            reason=scope.cancelled()=>reason,
            _=tokio::time::sleep_until(scope.deadline())=>{ scope.cancel("DEADLINE_EXCEEDED");"DEADLINE_EXCEEDED" },
        }
    };
    let result = tokio::select! {
        biased;
        result=&mut call=>match result { Ok(result)=>{ let result=if let Some(reason)=scope.reason() { Err(error(reason,"App call cancelled")) } else if tokio::time::Instant::now()>=scope.deadline() { Err(error("DEADLINE_EXCEEDED","App call deadline exceeded")) } else { result };return Completed { id,result,force:None } },Err(_)=>return Completed { id,result:Err(error("APP_TRAPPED","App trapped")),force:Some("APP_TRAPPED") } },
        reason=ending=>reason,
    };
    // A cancellation completes even if the guest later returns success.
    let force = tokio::select! {
        _=&mut call=>None,
        _=tokio::time::sleep(Duration::from_millis(limits.cancel_grace_ms))=>Some(result),
    };
    Completed {
        id,
        result: Err(error(result, "App call cancelled")),
        force,
    }
}
pub(super) async fn serve(
    accessor: &Accessor<Host>,
    worker: &Worker,
    ipc: &Ipc,
    incoming: &mut mpsc::Receiver<Message>,
    queued: &mut VecDeque<Message>,
    active: &mut HashMap<String, Active>,
    limits: Limits,
) -> wasmtime::Result<Option<(Option<String>, &'static str)>> {
    let attribution = accessor.with(|mut access| access.get().attribution.clone());
    let mut calls: FuturesUnordered<BoxFuture<'_, Completed>> = FuturesUnordered::new();
    for entry in active.values() {
        calls.push(Box::pin(invoke(
            accessor,
            worker,
            entry.request.clone(),
            entry.scope.clone(),
            limits,
            &attribution,
        )));
    }
    loop {
        tokio::select! {
            biased;
            _=ipc.closed()=>return Ok(None),
            Some(done)=calls.next(),if !calls.is_empty()=> {
                if let Some(code)=done.force { return Ok(Some((if code=="APP_TRAPPED" { attribution.culprit() } else { Some(done.id) },code))); }
                if let Some(entry)=active.remove(&done.id) { entry.scope.finish(); ipc.respond(done.id,done.result).await?; }
            },
            message=async { if let Some(message)=queued.pop_front() { Some(message) } else { incoming.recv().await } }=>match message {
                None=>return Ok(None),
                Some(Message::Cancel { id,.. })=>if let Some(entry)=active.get_mut(&id) { entry.cancellation=Some("CANCELLED");entry.scope.cancel("CANCELLED"); },
                Some(Message::Request { id,method,params,deadline_ms,context,.. })=> {
                    if active.contains_key(&id) { return Err(wasmtime::Error::msg("duplicate active App call id")); }
                    if active.len()>=MAX_CALLS { ipc.respond(id,Err(error("APP_BUSY","App has 64 calls in flight"))).await?;continue; }
                    let request=match Request::parse(&method,params,deadline_ms,context,limits) { Ok(value)=>value,Err(error)=>{ ipc.respond(id,Err(error)).await?;continue; } };
                    if matches!(request.operation,Operation::LiveEvent { .. }) && active.values().filter(|entry|matches!(entry.request.operation,Operation::LiveEvent { .. })).count()>=16 {
                        ipc.respond(id,Err(error("APP_BUSY","App has 16 live event deliveries in flight"))).await?;continue;
                    }
                    let scope=Scope::new(id.clone(),request.deadline);
                    calls.push(Box::pin(invoke(accessor,worker,request.clone(),scope.clone(),limits,&attribution)));
                    active.insert(id,Active { request,scope,attempts:0,cancellation:None });
                },
                Some(_)=>return Err(wasmtime::Error::msg("unexpected App IPC message")),
            },
        }
    }
}

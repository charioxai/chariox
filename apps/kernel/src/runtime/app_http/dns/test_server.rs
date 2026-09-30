//! In-process UDP name server for tests. It answers A and AAAA queries from a
//! script and counts them, so address policy and connection pinning run
//! through the production Hickory resolver instead of a stubbed lookup.
use super::DnsConfig;
use hickory_resolver::proto::{
    op::{Message, ResponseCode},
    rr::{
        rdata::{A, AAAA},
        RData, Record, RecordType,
    },
};
use std::{
    collections::HashMap,
    net::{IpAddr, SocketAddr},
    sync::{Arc, Mutex},
};
use tokio::{
    net::UdpSocket,
    task::JoinHandle,
    time::{Duration, Instant},
};

/// Answers for one name: `(name, record type, earlier queries of that type
/// for that name)`. `None` is NXDOMAIN; addresses of the other family are
/// left out, so an empty answer is NOERROR without data.
type Script = dyn Fn(&str, RecordType, usize) -> Option<Vec<IpAddr>> + Send + Sync;

enum Behavior {
    /// Answers from the script, except queries of one record type, which it
    /// counts and never replies to.
    Answer(Arc<Script>, Option<RecordType>),
    /// Counts queries and never replies, like a blackholed server.
    Silent,
    /// Replies to queries of one record type (or all) with this error code;
    /// other types get an answer without data.
    Fail(ResponseCode, Option<RecordType>),
}

pub(in crate::runtime::app_http) struct TestDns {
    address: SocketAddr,
    queries: Arc<Mutex<HashMap<(String, RecordType), usize>>>,
    arrivals: Arc<Mutex<Vec<Instant>>>,
    task: JoinHandle<()>,
}

impl TestDns {
    pub(in crate::runtime::app_http) async fn start(
        script: impl Fn(&str, RecordType, usize) -> Option<Vec<IpAddr>> + Send + Sync + 'static,
    ) -> Self {
        Self::serve(Behavior::Answer(Arc::new(script), None)).await
    }

    /// Answers from the script but never replies to queries of `kind`.
    pub(in crate::runtime::app_http) async fn dropping(
        kind: RecordType,
        script: impl Fn(&str, RecordType, usize) -> Option<Vec<IpAddr>> + Send + Sync + 'static,
    ) -> Self {
        Self::serve(Behavior::Answer(Arc::new(script), Some(kind))).await
    }

    /// A server that receives queries and never answers them.
    pub(in crate::runtime::app_http) async fn silent() -> Self {
        Self::serve(Behavior::Silent).await
    }

    /// A server that answers every query with an error code.
    pub(in crate::runtime::app_http) async fn failing(code: ResponseCode) -> Self {
        Self::serve(Behavior::Fail(code, None)).await
    }

    /// A server that answers one record type with an error code, and every
    /// other type without data.
    pub(in crate::runtime::app_http) async fn failing_only(
        code: ResponseCode,
        kind: RecordType,
    ) -> Self {
        Self::serve(Behavior::Fail(code, Some(kind))).await
    }

    async fn serve(behavior: Behavior) -> Self {
        let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let address = socket.local_addr().unwrap();
        let queries = Arc::new(Mutex::new(HashMap::new()));
        let seen = queries.clone();
        let arrivals = Arc::new(Mutex::new(Vec::new()));
        let arrived = arrivals.clone();
        let task = tokio::spawn(async move {
            let mut buffer = vec![0u8; 4096];
            loop {
                let Ok((length, peer)) = socket.recv_from(&mut buffer).await else {
                    return;
                };
                arrived.lock().unwrap().push(Instant::now());
                let Ok(request) = Message::from_vec(&buffer[..length]) else {
                    continue;
                };
                let Some(reply) = answer(&request, &seen, &behavior) else {
                    continue;
                };
                let _ = socket.send_to(&reply.to_vec().unwrap(), peer).await;
            }
        });
        Self {
            address,
            queries,
            arrivals,
            task,
        }
    }

    pub(in crate::runtime::app_http) fn address(&self) -> SocketAddr {
        self.address
    }

    pub(in crate::runtime::app_http) fn config(&self) -> DnsConfig {
        DnsConfig::fixture(self.address)
    }

    /// How many queries of this type the server received for an absolute name.
    pub(in crate::runtime::app_http) fn queries(&self, name: &str, kind: RecordType) -> usize {
        self.queries
            .lock()
            .unwrap()
            .get(&(name.to_ascii_lowercase(), kind))
            .copied()
            .unwrap_or(0)
    }

    pub(in crate::runtime::app_http) fn total_queries(&self) -> usize {
        self.queries.lock().unwrap().values().sum()
    }

    /// Whether any query arrived in `[from, to)` after `start`.
    pub(in crate::runtime::app_http) fn asked_between(
        &self,
        start: Instant,
        from: Duration,
        to: Duration,
    ) -> bool {
        self.arrivals
            .lock()
            .unwrap()
            .iter()
            .any(|at| (from..to).contains(&at.duration_since(start)))
    }
}

impl Drop for TestDns {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn answer(
    request: &Message,
    seen: &Mutex<HashMap<(String, RecordType), usize>>,
    behavior: &Behavior,
) -> Option<Message> {
    let mut reply = Message::response(request.metadata.id, request.metadata.op_code);
    reply.metadata.recursion_desired = request.metadata.recursion_desired;
    reply.metadata.recursion_available = true;
    reply.edns = request.edns.clone();
    reply.add_queries(request.queries.clone());
    let Some(query) = request.queries.first() else {
        reply.metadata.response_code = ResponseCode::FormErr;
        return Some(reply);
    };
    let name = query.name().to_ascii().to_ascii_lowercase();
    let kind = query.query_type();
    let sequence = {
        let mut seen = seen.lock().unwrap();
        let count = seen.entry((name.clone(), kind)).or_insert(0);
        *count += 1;
        *count - 1
    };
    let script = match behavior {
        Behavior::Answer(_, Some(dropped)) if *dropped == kind => return None,
        Behavior::Answer(script, _) => script,
        Behavior::Silent => return None,
        Behavior::Fail(code, only) => {
            if only.is_none_or(|only| only == kind) {
                reply.metadata.response_code = *code;
            }
            return Some(reply);
        }
    };
    match script(&name, kind, sequence) {
        None => reply.metadata.response_code = ResponseCode::NXDomain,
        Some(addresses) => {
            for address in addresses {
                let data = match (kind, address) {
                    (RecordType::A, IpAddr::V4(address)) => RData::A(A(address)),
                    (RecordType::AAAA, IpAddr::V6(address)) => RData::AAAA(AAAA(address)),
                    _ => continue,
                };
                reply.add_answer(Record::from_rdata(query.name().clone(), 0, data));
            }
        }
    }
    Some(reply)
}

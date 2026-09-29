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
use tokio::{net::UdpSocket, task::JoinHandle};

/// Answers for one name: `(name, record type, earlier queries of that type
/// for that name)`. `None` is NXDOMAIN; addresses of the other family are
/// left out, so an empty answer is NOERROR without data.
type Script = dyn Fn(&str, RecordType, usize) -> Option<Vec<IpAddr>> + Send + Sync;

pub(in crate::runtime::app_http) struct TestDns {
    address: SocketAddr,
    queries: Arc<Mutex<HashMap<(String, RecordType), usize>>>,
    task: JoinHandle<()>,
}

impl TestDns {
    pub(in crate::runtime::app_http) async fn start(
        script: impl Fn(&str, RecordType, usize) -> Option<Vec<IpAddr>> + Send + Sync + 'static,
    ) -> Self {
        let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let address = socket.local_addr().unwrap();
        let queries = Arc::new(Mutex::new(HashMap::new()));
        let seen = queries.clone();
        let script: Arc<Script> = Arc::new(script);
        let task = tokio::spawn(async move {
            let mut buffer = vec![0u8; 4096];
            loop {
                let Ok((length, peer)) = socket.recv_from(&mut buffer).await else {
                    return;
                };
                let Ok(request) = Message::from_vec(&buffer[..length]) else {
                    continue;
                };
                let reply = answer(&request, &seen, script.as_ref());
                let _ = socket.send_to(&reply.to_vec().unwrap(), peer).await;
            }
        });
        Self {
            address,
            queries,
            task,
        }
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
}

impl Drop for TestDns {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn answer(
    request: &Message,
    seen: &Mutex<HashMap<(String, RecordType), usize>>,
    script: &Script,
) -> Message {
    let mut reply = Message::response(request.metadata.id, request.metadata.op_code);
    reply.metadata.recursion_desired = request.metadata.recursion_desired;
    reply.metadata.recursion_available = true;
    reply.edns = request.edns.clone();
    reply.add_queries(request.queries.clone());
    let Some(query) = request.queries.first() else {
        reply.metadata.response_code = ResponseCode::FormErr;
        return reply;
    };
    let name = query.name().to_ascii().to_ascii_lowercase();
    let kind = query.query_type();
    let sequence = {
        let mut seen = seen.lock().unwrap();
        let count = seen.entry((name.clone(), kind)).or_insert(0);
        *count += 1;
        *count - 1
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
    reply
}

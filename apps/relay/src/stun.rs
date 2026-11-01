//! MP-08/MP-11: a STUN Binding responder (RFC 8489) on the relay host.
//!
//! A browser and a kernel behind NAT each ask it for the public address their
//! UDP socket maps to, so the WebRTC data channel can reach between them
//! directly. It answers Binding requests only: no TURN, no allocations, no
//! state, and it never sees session content.

use std::net::{IpAddr, SocketAddr};

use tokio::net::UdpSocket;

pub const DEFAULT_PORT: u16 = 3478;

const MAGIC_COOKIE: [u8; 4] = [0x21, 0x12, 0xa4, 0x42];
const HEADER_BYTES: usize = 20;
const BINDING_REQUEST: u16 = 0x0001;
const BINDING_SUCCESS: u16 = 0x0101;
const XOR_MAPPED_ADDRESS: u16 = 0x0020;
const FAMILY_V4: u8 = 0x01;
const FAMILY_V6: u8 = 0x02;

/// A Binding request with this transaction id and no attributes.
pub fn binding_request(transaction: [u8; 12]) -> Vec<u8> {
    let mut message = Vec::with_capacity(HEADER_BYTES);
    message.extend_from_slice(&BINDING_REQUEST.to_be_bytes());
    message.extend_from_slice(&0_u16.to_be_bytes());
    message.extend_from_slice(&MAGIC_COOKIE);
    message.extend_from_slice(&transaction);
    message
}

/// The Binding success response telling `source` its own address, or `None`
/// when `request` is not a Binding request.
pub fn binding_response(request: &[u8], source: SocketAddr) -> Option<Vec<u8>> {
    let (kind, transaction) = header(request)?;
    if kind != BINDING_REQUEST {
        return None;
    }
    let mut value = vec![0, 0];
    let port = source.port() ^ u16::from_be_bytes([MAGIC_COOKIE[0], MAGIC_COOKIE[1]]);
    value.extend_from_slice(&port.to_be_bytes());
    match source.ip() {
        IpAddr::V4(ip) => {
            value[1] = FAMILY_V4;
            value.extend(xor(&ip.octets(), &transaction));
        }
        IpAddr::V6(ip) => {
            value[1] = FAMILY_V6;
            value.extend(xor(&ip.octets(), &transaction));
        }
    }
    let mut response = Vec::with_capacity(HEADER_BYTES + 4 + value.len());
    response.extend_from_slice(&BINDING_SUCCESS.to_be_bytes());
    response.extend_from_slice(&(4 + value.len() as u16).to_be_bytes());
    response.extend_from_slice(&MAGIC_COOKIE);
    response.extend_from_slice(&transaction);
    response.extend_from_slice(&XOR_MAPPED_ADDRESS.to_be_bytes());
    response.extend_from_slice(&(value.len() as u16).to_be_bytes());
    response.extend_from_slice(&value);
    Some(response)
}

/// The mapped address in a Binding success response to `transaction`.
pub fn mapped_address(response: &[u8], transaction: [u8; 12]) -> Option<SocketAddr> {
    let (kind, received) = header(response)?;
    if kind != BINDING_SUCCESS || received != transaction {
        return None;
    }
    let mut attributes = response.get(HEADER_BYTES..)?;
    while attributes.len() >= 4 {
        let kind = u16::from_be_bytes([attributes[0], attributes[1]]);
        let len = usize::from(u16::from_be_bytes([attributes[2], attributes[3]]));
        let value = attributes.get(4..4 + len)?;
        if kind == XOR_MAPPED_ADDRESS && len >= 8 {
            let port = u16::from_be_bytes([value[2], value[3]])
                ^ u16::from_be_bytes([MAGIC_COOKIE[0], MAGIC_COOKIE[1]]);
            let ip = match (value[1], len) {
                (FAMILY_V4, 8) => {
                    IpAddr::from(<[u8; 4]>::try_from(xor(&value[4..8], &transaction)).ok()?)
                }
                (FAMILY_V6, 20) => {
                    IpAddr::from(<[u8; 16]>::try_from(xor(&value[4..20], &transaction)).ok()?)
                }
                _ => return None,
            };
            return Some(SocketAddr::new(ip, port));
        }
        attributes = attributes.get(4 + len.next_multiple_of(4)..)?;
    }
    None
}

/// The transaction id of a well-formed STUN message (the first two bits zero
/// and the magic cookie, which DTLS and SRTP packets never have).
pub fn transaction(message: &[u8]) -> Option<[u8; 12]> {
    header(message).map(|(_, transaction)| transaction)
}

fn header(message: &[u8]) -> Option<(u16, [u8; 12])> {
    if message.len() < HEADER_BYTES || message[0] & 0xc0 != 0 || message[4..8] != MAGIC_COOKIE {
        return None;
    }
    let len = usize::from(u16::from_be_bytes([message[2], message[3]]));
    if HEADER_BYTES + len != message.len() || len % 4 != 0 {
        return None;
    }
    let kind = u16::from_be_bytes([message[0], message[1]]);
    Some((kind, message[8..20].try_into().ok()?))
}

/// IPv4 is XORed with the cookie; IPv6 with the cookie and transaction id.
fn xor(octets: &[u8], transaction: &[u8; 12]) -> Vec<u8> {
    MAGIC_COOKIE
        .iter()
        .chain(transaction)
        .zip(octets)
        .map(|(key, octet)| key ^ octet)
        .collect()
}

/// Answers Binding requests on `socket` for as long as the relay runs. A
/// receive error is one datagram's (an ICMP unreachable from an earlier
/// reply, say), and a lost reply is retried by the asker.
pub async fn serve(socket: UdpSocket) {
    let mut buf = [0_u8; 1500];
    loop {
        let Ok((len, source)) = socket.recv_from(&mut buf).await else {
            continue;
        };
        if let Some(response) = binding_response(&buf[..len], source) {
            let _ = socket.send_to(&response, source).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TRANSACTION: [u8; 12] = *b"chariox-stun";

    #[test]
    fn mp08_binding_response_maps_ipv4_and_ipv6_sources() {
        for source in ["203.0.113.7:51234", "[2001:db8::7]:443"] {
            let source: SocketAddr = source.parse().unwrap();
            let response = binding_response(&binding_request(TRANSACTION), source).unwrap();
            assert_eq!(mapped_address(&response, TRANSACTION), Some(source));
            assert_eq!(transaction(&response), Some(TRANSACTION));
            assert_eq!(mapped_address(&response, *b"another-txid"), None);
        }
    }

    #[test]
    fn mp08_responder_ignores_everything_but_binding_requests() {
        let source: SocketAddr = "203.0.113.7:51234".parse().unwrap();
        let response = binding_response(&binding_request(TRANSACTION), source).unwrap();
        assert_eq!(binding_response(&response, source), None);
        assert_eq!(binding_response(&[0x16, 0xfe, 0xfd, 0, 0], source), None);
        let mut no_cookie = binding_request(TRANSACTION);
        no_cookie[4] = 0;
        assert_eq!(binding_response(&no_cookie, source), None);
        let mut long = binding_request(TRANSACTION);
        long.extend_from_slice(&[0; 4]);
        assert_eq!(binding_response(&long, source), None);
    }

    /// Chrome's Binding requests carry attributes (SOFTWARE, FINGERPRINT); a
    /// real request from the wire must still be answered.
    #[test]
    fn mp08_binding_request_with_attributes_is_answered() {
        let mut request = binding_request(TRANSACTION);
        request.extend_from_slice(&[0x80, 0x28, 0, 4, 1, 2, 3, 4]);
        request[3] = 8;
        let source: SocketAddr = "198.51.100.1:9".parse().unwrap();
        let response = binding_response(&request, source).unwrap();
        assert_eq!(mapped_address(&response, TRANSACTION), Some(source));
    }

    #[tokio::test]
    async fn mp08_responder_answers_over_udp() {
        let server = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let address = server.local_addr().unwrap();
        let task = tokio::spawn(serve(server));
        let client = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        client
            .send_to(&binding_request(TRANSACTION), address)
            .await
            .unwrap();
        let mut buf = [0_u8; 256];
        let (len, _) = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            client.recv_from(&mut buf),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(
            mapped_address(&buf[..len], TRANSACTION),
            Some(client.local_addr().unwrap())
        );
        task.abort();
    }
}

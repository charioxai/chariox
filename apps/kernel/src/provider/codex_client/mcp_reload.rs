//! Official in-place refresh on the kernel-owned local app-server. One absolute
//! deadline covers TCP, WebSocket upgrade and every read/write, including peers
//! that trickle an incomplete handshake or frame. No thread or turn is created.
use super::CodexClient;
use crate::error::DaemonError;
use serde_json::{json, Value};
use std::{
    io::{self, Read, Write},
    net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream},
    time::{Duration, Instant},
};
use tokio_tungstenite::tungstenite::{client::client_with_config, Message, WebSocket};

#[cfg(test)]
mod tests;

impl CodexClient {
    pub(crate) fn reload_mcp_servers_until(&self, deadline: Instant) -> Result<(), DaemonError> {
        let result = self.reload_mcp_until(deadline);
        result.map_err(|error| self.protocol_error("codex_mcp_reload", error.to_string()))
    }

    fn reload_mcp_until(&self, deadline: Instant) -> io::Result<()> {
        let url = url::Url::parse(&self.endpoint).map_err(|_| invalid("invalid local endpoint"))?;
        let ip: IpAddr = match url.host() {
            Some(url::Host::Ipv4(ip)) => ip.into(),
            Some(url::Host::Ipv6(ip)) => ip.into(),
            Some(url::Host::Domain("localhost")) => Ipv4Addr::LOCALHOST.into(),
            _ => {
                return Err(invalid(
                    "refresh requires a local numeric app-server endpoint",
                ))
            }
        };
        if url.scheme() != "ws"
            || !ip.is_loopback()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || url.path() != "/"
        {
            return Err(invalid(
                "refresh requires a kernel-owned loopback WebSocket",
            ));
        }
        let address = SocketAddr::new(
            ip,
            url.port_or_known_default()
                .ok_or_else(|| invalid("invalid app-server port"))?,
        );
        let stream = TcpStream::connect_timeout(&address, remaining(deadline)?)?;
        stream.set_nonblocking(true)?;
        let stream = DeadlineStream {
            stream,
            deadline,
            read_left: 262_144,
            write_left: 65_536,
        };
        let mut config = tokio_tungstenite::tungstenite::protocol::WebSocketConfig::default();
        config.max_message_size = Some(65_536);
        config.max_frame_size = Some(65_536);
        config.write_buffer_size = 0;
        config.max_write_buffer_size = 65_536;
        let (mut socket, _) = client_with_config(self.endpoint.as_str(), stream, Some(config))
            .map_err(|_| {
                invalid("app-server WebSocket handshake failed or exceeded its deadline")
            })?;
        send(
            &mut socket,
            json!({"jsonrpc":"2.0","id":0,"method":"initialize","params":{
                "clientInfo":{"name":"chariox-kernel","version":env!("CARGO_PKG_VERSION")},
                "capabilities":{"experimentalApi":true}
            }}),
        )?;
        response(&mut socket, 0)?;
        send(
            &mut socket,
            json!({"jsonrpc":"2.0","method":"initialized","params":{}}),
        )?;
        send(
            &mut socket,
            // Official McpServerRefresh uses Option<()> (undefined params),
            // not an empty object. See app-server-protocol/protocol/common.rs.
            json!({"jsonrpc":"2.0","id":1,"method":"config/mcpServer/reload"}),
        )?;
        response(&mut socket, 1)
    }
}

fn send(socket: &mut WebSocket<DeadlineStream>, value: Value) -> io::Result<()> {
    remaining(socket.get_ref().deadline)?;
    socket
        .send(Message::Text(value.to_string().into()))
        .map_err(|_| invalid("app-server refresh write failed"))
}
fn response(socket: &mut WebSocket<DeadlineStream>, id: u64) -> io::Result<()> {
    loop {
        remaining(socket.get_ref().deadline)?;
        let message = socket
            .read()
            .map_err(|_| invalid("app-server refresh response failed or exceeded its deadline"))?;
        let text = match message {
            Message::Text(text) => text,
            Message::Ping(_) | Message::Pong(_) => continue,
            _ => return Err(invalid("unexpected app-server refresh response")),
        };
        let value: Value = serde_json::from_str(&text)
            .map_err(|_| invalid("malformed app-server refresh response"))?;
        if value.get("id").and_then(Value::as_u64) != Some(id) {
            continue;
        }
        if value.get("error").is_some() || !value.get("result").is_some_and(Value::is_object) {
            return Err(invalid(
                "app-server rejected MCP refresh; check its supported protocol",
            ));
        }
        return Ok(());
    }
}
fn invalid(message: &'static str) -> io::Error {
    io::Error::other(message)
}
fn remaining(deadline: Instant) -> io::Result<Duration> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|duration| !duration.is_zero())
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::TimedOut,
                "native MCP refresh deadline elapsed",
            )
        })
}
struct DeadlineStream {
    stream: TcpStream,
    deadline: Instant,
    read_left: usize,
    write_left: usize,
}
impl DeadlineStream {
    fn retry(&self, error: io::Error) -> io::Result<()> {
        match error.kind() {
            io::ErrorKind::Interrupted => {
                remaining(self.deadline)?;
                Ok(())
            }
            io::ErrorKind::WouldBlock => {
                std::thread::sleep(remaining(self.deadline)?.min(Duration::from_millis(2)));
                Ok(())
            }
            _ => Err(error),
        }
    }
}
impl Read for DeadlineStream {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        loop {
            remaining(self.deadline)?;
            if bytes.is_empty() {
                return Ok(0);
            }
            if self.read_left == 0 {
                return Err(invalid("refresh response exceeded byte budget"));
            }
            let count = bytes.len().min(self.read_left);
            match self.stream.read(&mut bytes[..count]) {
                Ok(read) => {
                    self.read_left -= read;
                    return Ok(read);
                }
                Err(error) => self.retry(error)?,
            }
        }
    }
}
impl Write for DeadlineStream {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        loop {
            remaining(self.deadline)?;
            if bytes.len() > self.write_left {
                return Err(invalid("refresh request exceeded byte budget"));
            }
            match self.stream.write(bytes) {
                Ok(written) => {
                    self.write_left -= written;
                    return Ok(written);
                }
                Err(error) => self.retry(error)?,
            }
        }
    }
    fn flush(&mut self) -> io::Result<()> {
        remaining(self.deadline)?;
        self.stream.flush()
    }
}

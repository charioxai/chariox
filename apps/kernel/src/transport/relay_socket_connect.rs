//! MP-08 / MP-10 / MP-11: bound the stack used to construct relay TLS futures.
use std::{future::Future, pin::Pin};

use tokio::net::TcpStream;
use tokio_tungstenite::{
    connect_async_with_config,
    tungstenite::{handshake::client::Response, Error},
    MaybeTlsStream, WebSocketStream,
};

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;
type ConnectFuture<'a> =
    Pin<Box<dyn Future<Output = Result<(Socket, Response), Error>> + Send + 'a>>;

// Keep construction out of callers' poll frames, including their timeout and
// cancellation wrappers. The normal relay connection remains deferred.
#[inline(never)]
pub(super) fn connect(relay_url: &str) -> ConnectFuture<'_> {
    Box::pin(connect_async_with_config(
        chariox_relay::frame_transport::transport_url(relay_url),
        None,
        std::env::var("CHARIOX_KERNEL_BROWSER_DISPLAY").as_deref() == Ok("1"),
    ))
}

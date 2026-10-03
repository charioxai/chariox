use tokio::net::TcpStream;

pub(super) fn configure(stream: TcpStream) -> Option<TcpStream> {
    if let Err(error) = stream.set_nodelay(true) {
        crate::logging::warn_with_fields(
            "daemon.runtime_transport",
            "failed configuring kernel websocket TCP_NODELAY",
            serde_json::json!({ "error": error.to_string() }),
        );
        return None;
    }
    Some(stream)
}

#[cfg(test)]
mod tests {
    use super::configure;
    use tokio::net::{TcpListener, TcpStream};

    async fn accepted_stream() -> (TcpStream, TcpStream) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let (client, (server, _)) = tokio::try_join!(
            TcpStream::connect(listener.local_addr().unwrap()),
            listener.accept(),
        )
        .unwrap();
        (client, server)
    }

    #[tokio::test]
    async fn accepted_stream_disables_nagle() {
        let (_client, server) = accepted_stream().await;
        assert!(!server.nodelay().unwrap());
        let server = configure(server).expect("accepted TCP stream");
        assert!(server.nodelay().unwrap());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn unsupported_option_drops_only_that_stream() {
        use std::io::Read;
        use std::os::fd::OwnedFd;
        use std::os::unix::net::UnixStream;

        let (socket, mut peer) = UnixStream::pair().unwrap();
        socket.set_nonblocking(true).unwrap();
        let fd: OwnedFd = socket.into();
        // A Unix socket is pollable but rejects the TCP-only option.
        let stream = TcpStream::from_std(std::net::TcpStream::from(fd)).unwrap();
        assert!(configure(stream).is_none());
        assert_eq!(peer.read(&mut [0]).unwrap(), 0);

        let (_client, server) = accepted_stream().await;
        assert!(configure(server).unwrap().nodelay().unwrap());
    }
}

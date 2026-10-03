//! Bounded private root-helper protocol; never a public daemon/App RPC. The
//! connected OS peer supplies identity, and SCM_RIGHTS is rejected and closed.
use super::{Error, Result};
use serde::{Deserialize, Serialize};
use std::{
    io::Write,
    os::{fd::AsRawFd, unix::net::UnixStream},
    time::{Duration, Instant},
};
const MAX_FRAME: usize = 4096;

pub(super) fn peer(stream: &UnixStream) -> Result<u32> {
    let mut value = std::mem::MaybeUninit::<libc::ucred>::zeroed();
    let mut length = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    if unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            value.as_mut_ptr().cast(),
            &mut length,
        )
    } != 0
        || length as usize != std::mem::size_of::<libc::ucred>()
    {
        return Err(Error::Identity);
    }
    let value = unsafe { value.assume_init() };
    if value.pid <= 0 {
        return Err(Error::Identity);
    }
    Ok(value.uid)
}
#[derive(Default)]
pub(super) struct Reader {
    bytes: Vec<u8>,
    length: Option<usize>,
}
impl Reader {
    /// At most one bounded receive per pass. No allocating from an unchecked
    /// length header and no extra request pipelining around lease ownership.
    pub fn next(&mut self, stream: &UnixStream) -> Result<Option<Vec<u8>>> {
        let target = self.length.map(|v| v + 4).unwrap_or(4);
        if self.bytes.len() == target {
            if self.length.is_some() {
                return Ok(Some(self.bytes.split_off(4)));
            }
            let length = u32::from_be_bytes(self.bytes[..4].try_into().unwrap()) as usize;
            if length == 0 || length > MAX_FRAME {
                return Err(Error::Invalid);
            }
            self.length = Some(length);
        }
        let target = self.length.map(|v| v + 4).unwrap_or(4);
        let mut bytes = [0u8; MAX_FRAME];
        let mut io = libc::iovec {
            iov_base: bytes.as_mut_ptr().cast(),
            iov_len: (target - self.bytes.len()).min(bytes.len()),
        };
        // Aligned buffer large enough for 16 descriptors; excess rights are
        // closed by Linux on MSG_CTRUNC, observed descriptors are closed below.
        let mut control = [0usize; 32];
        let mut message: libc::msghdr = unsafe { std::mem::zeroed() };
        message.msg_iov = &mut io;
        message.msg_iovlen = 1;
        message.msg_control = control.as_mut_ptr().cast();
        message.msg_controllen = std::mem::size_of_val(&control);
        let count = unsafe {
            libc::recvmsg(
                stream.as_raw_fd(),
                &mut message,
                libc::MSG_DONTWAIT | libc::MSG_CMSG_CLOEXEC,
            )
        };
        if count < 0 {
            return match std::io::Error::last_os_error().kind() {
                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted => Ok(None),
                _ => Err(Error::Io),
            };
        }
        if message.msg_controllen != 0 || message.msg_flags & libc::MSG_CTRUNC != 0 {
            unsafe {
                let mut header = libc::CMSG_FIRSTHDR(&message);
                while !header.is_null() {
                    if (*header).cmsg_level == libc::SOL_SOCKET
                        && (*header).cmsg_type == libc::SCM_RIGHTS
                    {
                        let size = (*header)
                            .cmsg_len
                            .saturating_sub(libc::CMSG_LEN(0) as usize);
                        let data = libc::CMSG_DATA(header).cast::<i32>();
                        for index in 0..size / std::mem::size_of::<i32>() {
                            libc::close(*data.add(index));
                        }
                    }
                    header = libc::CMSG_NXTHDR(&message, header);
                }
            }
            return Err(Error::Invalid);
        }
        if count == 0 {
            return Err(Error::Io);
        }
        self.bytes.extend_from_slice(&bytes[..count as usize]);
        if self.length.is_some() && self.bytes.len() == target {
            return Ok(Some(self.bytes.split_off(4)));
        }
        Ok(None)
    }
}
pub(super) fn send<T: Serialize>(stream: &mut UnixStream, value: &T) -> Result<()> {
    let bytes = serde_json::to_vec(value).map_err(|_| Error::Invalid)?;
    if bytes.is_empty() || bytes.len() > MAX_FRAME {
        return Err(Error::Invalid);
    }
    let frame = [(bytes.len() as u32).to_be_bytes().as_slice(), &bytes].concat();
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut sent = 0;
    while sent < frame.len() {
        match stream.write(&frame[sent..]) {
            Ok(0) => return Err(Error::Io),
            Ok(count) => sent += count,
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
                ) => {}
            Err(_) => return Err(Error::Io),
        }
        if Instant::now() >= deadline {
            return Err(Error::Busy);
        }
        if sent < frame.len() {
            poll(stream, libc::POLLOUT)?;
        }
    }
    Ok(())
}
pub(super) fn receive<T: for<'a> Deserialize<'a>>(stream: &UnixStream, seconds: u64) -> Result<T> {
    let deadline = Instant::now() + Duration::from_secs(seconds);
    let mut reader = Reader::default();
    loop {
        if let Some(bytes) = reader.next(stream)? {
            return serde_json::from_slice(&bytes).map_err(|_| Error::Invalid);
        }
        if Instant::now() >= deadline {
            return Err(Error::Busy);
        }
        poll(stream, libc::POLLIN)?;
    }
}
fn poll(stream: &UnixStream, events: i16) -> Result<()> {
    let mut value = libc::pollfd {
        fd: stream.as_raw_fd(),
        events,
        revents: 0,
    };
    if unsafe { libc::poll(&mut value, 1, 20) } < 0
        && std::io::Error::last_os_error().kind() != std::io::ErrorKind::Interrupted
    {
        return Err(Error::Io);
    }
    Ok(())
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Reply {
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grant: Option<super::store::Grant>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<super::code_model::Grant>,
    /// With `app_storage_host_reserve` only: what is free and needed, for the
    /// owner. A client that predates it refuses the reply; the start fails
    /// either way.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_disk: Option<crate::worker_process::HostDiskSpace>,
}
impl Reply {
    pub fn failed(error: Error) -> Self {
        Self {
            status: error.to_string(),
            grant: None,
            code: None,
            host_disk: match error {
                Error::HostReserve(space) => Some(space),
                _ => None,
            },
        }
    }
    pub fn released() -> Self {
        Self {
            status: "released".into(),
            grant: None,
            code: None,
            host_disk: None,
        }
    }
    pub fn deleted() -> Self {
        Self {
            status: "deleted".into(),
            grant: None,
            code: None,
            host_disk: None,
        }
    }
    /// A refused acquire, as the helper reported it: short of disk space
    /// with its numbers, or any other refusal (recovery decides).
    pub fn refusal(&self) -> Error {
        match self.host_disk {
            Some(space) if self.status == Error::HostReserve(space).to_string() => {
                Error::HostReserve(space)
            }
            _ => Error::RecoveryRequired,
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fragmented_frames_are_bounded_before_allocating_or_decoding() {
        let (mut writer, reader) = UnixStream::pair().unwrap();
        writer.set_nonblocking(true).unwrap();
        reader.set_nonblocking(true).unwrap();
        let mut state = Reader::default();
        for byte in [0, 0, 0, 2, b'{', b'}'] {
            writer.write_all(&[byte]).unwrap();
            let value = state.next(&reader).unwrap();
            if byte == b'}' {
                assert_eq!(value.unwrap(), b"{}");
            } else {
                assert!(value.is_none());
            }
        }
        let mut state = Reader::default();
        writer.write_all(&4097u32.to_be_bytes()).unwrap();
        assert!(state.next(&reader).unwrap().is_none());
        assert_eq!(state.next(&reader), Err(Error::Invalid));
    }
    #[test]
    fn a_host_reserve_refusal_carries_what_is_free_and_needed() {
        let space = crate::worker_process::HostDiskSpace { free: 1, needed: 2 };
        let reply = Reply::failed(Error::HostReserve(space));
        let bytes = serde_json::to_vec(&reply).unwrap();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&bytes).unwrap(),
            serde_json::json!({"status": "app_storage_host_reserve", "host_disk": {"free": 1, "needed": 2}})
        );
        let received: Reply = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(received.refusal(), Error::HostReserve(space));
        // Other refusals, and numbers without the status, stay generic.
        assert_eq!(
            Reply::failed(Error::Capacity).refusal(),
            Error::RecoveryRequired
        );
        let forged = Reply {
            status: "app_storage_capacity".into(),
            ..received
        };
        assert_eq!(forged.refusal(), Error::RecoveryRequired);
        assert!(serde_json::to_string(&Reply::failed(Error::Capacity))
            .unwrap()
            .contains("\"status\":\"app_storage_capacity\"}"));
    }
    #[test]
    fn host_reserve_refusal_survives_helper_framing_and_client_decode() {
        let (mut helper, client) = UnixStream::pair().unwrap();
        helper.set_nonblocking(true).unwrap();
        client.set_nonblocking(true).unwrap();
        let space = crate::worker_process::HostDiskSpace { free: 7, needed: 8 };
        send(&mut helper, &Reply::failed(Error::HostReserve(space))).unwrap();
        let reply: Reply = receive(&client, 150).unwrap();
        assert_eq!(reply.refusal(), Error::HostReserve(space));
    }

    #[test]
    fn old_helper_refusals_stay_fail_closed_and_success_shape_is_unchanged() {
        let old: Reply = serde_json::from_str(r#"{"status":"app_storage_capacity"}"#).unwrap();
        assert_eq!(old.refusal(), Error::RecoveryRequired);
        let missing: Reply =
            serde_json::from_str(r#"{"status":"app_storage_host_reserve"}"#).unwrap();
        assert_eq!(missing.refusal(), Error::RecoveryRequired);
        assert_eq!(
            serde_json::to_value(Reply::released()).unwrap(),
            serde_json::json!({"status":"released"})
        );
    }

    #[test]
    fn structured_host_headroom_reply_is_decoded_without_losing_numbers() {
        let bytes = br#"{"status":"app_storage_host_reserve","host_disk":{"free":7,"needed":8}}"#;
        let reply: Reply = serde_json::from_slice(bytes)
            .expect("numeric host headroom refusal must be understood");
        let encoded = serde_json::to_value(reply).unwrap();
        assert_eq!(
            encoded["host_disk"],
            serde_json::json!({"free": 7, "needed": 8})
        );
    }

    #[test]
    fn authenticated_uid_is_observed_from_socket_peer() {
        let (left, _) = UnixStream::pair().unwrap();
        assert_eq!(peer(&left).unwrap(), unsafe { libc::geteuid() });
    }
}

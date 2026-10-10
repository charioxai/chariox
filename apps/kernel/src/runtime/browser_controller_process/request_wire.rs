//! MP-08/MP-10/MP-11: serialize a controller command before taking the pipe write.
//! Retain the JSON-lines contract and the caller's request/actor/cancellation order.
use serde::Serialize;
use std::io::{self, Write};
pub(super) fn write_line(writer: &mut impl Write, value: &impl Serialize) -> io::Result<()> {
    let mut line = serde_json::to_vec(value).map_err(io::Error::other)?;
    line.push(b'\n');
    writer.write_all(&line)?;
    writer.flush()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[derive(Default)]
    struct Counted {
        bytes: Vec<u8>,
        writes: usize,
        flushes: usize,
    }
    impl Write for Counted {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.writes += 1;
            self.bytes.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            self.flushes += 1;
            Ok(())
        }
    }
    #[test]
    fn mp08_controller_frame_and_cancel_keep_one_complete_ordered_pipe_write() {
        let mut pipe = Counted::default();
        let request = serde_json::json!({"id":7,"method":"host.browser","params":{"op":"display_capture","generation":1,"subscription_id":"s","after_sequence":3},"protected_values":[]});
        write_line(&mut pipe, &request).unwrap();
        assert_eq!(pipe.writes, 1);
        assert_eq!(pipe.flushes, 1);
        let cancel =
            serde_json::json!({"id":8,"method":"browser.cancel","params":{"request_id":7}});
        write_line(&mut pipe, &cancel).unwrap();
        assert_eq!(pipe.writes, 2);
        assert_eq!(pipe.flushes, 2);
        let lines: Vec<_> = pipe.bytes.split(|b| *b == b'\n').collect();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(lines[0]).unwrap(),
            request
        );
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(lines[1]).unwrap(),
            cancel
        );
        assert!(lines[2].is_empty());
    }
    #[test]
    fn mp11_serialization_failure_sends_no_partial_controller_command() {
        struct Refused;
        impl Serialize for Refused {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                use serde::ser::SerializeMap;
                let mut map = serializer.serialize_map(Some(2))?;
                map.serialize_entry("first", &1)?;
                Err(serde::ser::Error::custom("refused"))
            }
        }
        let mut pipe = Counted::default();
        assert!(write_line(&mut pipe, &Refused).is_err());
        assert!(pipe.bytes.is_empty());
        assert_eq!(pipe.writes, 0);
    }
}

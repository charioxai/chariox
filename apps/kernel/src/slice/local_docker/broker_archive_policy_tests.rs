use super::*;

fn exchange(
    request: &BrokerRequest<'_>,
    valid: bool,
) -> (bool, Option<Duration>, Option<Duration>) {
    let (writer, mut server) = UnixStream::pair().unwrap();
    writer
        .set_read_timeout(Some(Duration::from_millis(50)))
        .unwrap();
    writer
        .set_write_timeout(Some(Duration::from_millis(75)))
        .unwrap();
    let probe = writer.try_clone().unwrap();
    let reader = writer.try_clone().unwrap();
    let server = std::thread::spawn(move || {
        server
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut header = [0; 4];
        server.read_exact(&mut header).unwrap();
        let mut body = vec![0; u32::from_be_bytes(header) as usize];
        server.read_exact(&mut body).unwrap();
        std::thread::sleep(Duration::from_millis(120));
        let response = if valid {
            br#"{"status":0,"stdoutBase64":"","stderrBase64":""}"#.as_slice()
        } else {
            b"malformed".as_slice()
        };
        let _ = server.write_all(&(response.len() as u32).to_be_bytes());
        let _ = server.write_all(response);
    });
    let state = BROKER.get_or_init(|| Mutex::new(None));
    *state.lock().unwrap() = Some(BrokerConnection {
        reader: BufReader::new(reader),
        writer,
    });
    let result = execute_with_disk_evidence(request).is_ok();
    let read = probe.read_timeout().unwrap();
    let write = probe.write_timeout().unwrap();
    *state.lock().unwrap() = None;
    server.join().unwrap();
    (result, read, write)
}

#[test]
fn archive_policy_response_deadlines_preserve_healthy_archive_progress() {
    let _lock = crate::env_lock::lock();
    for request in [
        BrokerRequest::HomeArchiveCapture {
            container: "owned-helper",
            scope: "state",
            id: "owned",
        },
        BrokerRequest::HomeArchiveVerify {
            scope: "state",
            id: "owned",
            path: "/private/owned/home.tar.zst",
        },
    ] {
        let (success, read, write) = exchange(&request, true);
        assert!(
            success,
            "healthy archive response must outlive the generic deadline"
        );
        assert_eq!(read, Some(Duration::from_millis(50)));
        assert_eq!(write, Some(Duration::from_millis(75)));
        let (success, read, write) = exchange(&request, false);
        assert!(!success);
        assert_eq!(read, Some(Duration::from_millis(50)));
        assert_eq!(write, Some(Duration::from_millis(75)));
    }
    let (success, read, write) = exchange(&BrokerRequest::Docker { args: &[] }, true);
    assert!(
        !success,
        "unrelated commands retain their original deadline"
    );
    assert_eq!(read, Some(Duration::from_millis(50)));
    assert_eq!(write, Some(Duration::from_millis(75)));
}

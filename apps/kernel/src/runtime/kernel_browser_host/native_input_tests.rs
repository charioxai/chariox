//! MP-08/MP-10/MP-11: opt-in real DOM and native CDP regressions for PR #880.
use super::*;
use serde_json::json;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, Ordering};

struct NativeInput {
    host: KernelBrowserHost,
    stop: Arc<AtomicBool>,
    server: Option<std::thread::JoinHandle<()>>,
    tab: Value,
}

impl NativeInput {
    fn new(markup: &str) -> Self {
        assert_ne!(
            unsafe { libc::geteuid() },
            0,
            "MP-10: normal Unix user required"
        );
        let root = PathBuf::from(std::env::var_os("CHARIOX_MDACCESS_DRILL_ROOT").unwrap());
        assert!(root.is_absolute());
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}/", listener.local_addr().unwrap());
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        let body = format!(
            r#"<!doctype html><html><body>{markup}
<p id="result">submitted=0 releases=0 focus=field</p>
<script>
let submitted = 0, releases = 0;
const status = () => document.getElementById('result').textContent =
  `submitted=${{submitted}} releases=${{releases}} focus=${{document.activeElement.id}}`;
document.addEventListener('submit', event => {{ event.preventDefault(); submitted++; status(); }});
document.addEventListener('keyup', event => {{ if (event.key === 'Tab') releases++; status(); }});
document.getElementById('field').focus();
</script></body></html>"#
        );
        let server = std::thread::spawn(move || {
            while !stopping.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((mut socket, _)) => {
                        socket
                            .set_read_timeout(Some(Duration::from_secs(1)))
                            .unwrap();
                        let mut request = [0u8; 4096];
                        let _ = socket.read(&mut request);
                        let response = format!("HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body);
                        let _ = socket.write_all(response.as_bytes());
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Err(error) => panic!("MP-10: fixture listener failed: {error}"),
                }
            }
        });
        let mut fixture = Self {
            host: KernelBrowserHost::new(
                root.join(format!("input-{:032x}", rand::random::<u128>())),
            ),
            stop,
            server: Some(server),
            tab: Value::Null,
        };
        fixture.host.set_focus("owner", Some("first"));
        fixture.host.load("owner", "first").unwrap();
        let state = fixture.request(json!({"op":"open","url":url})).unwrap();
        fixture.tab = state["tabs"][0].clone();
        fixture.tab["generation"] = state["generation"].clone();
        // A real native click establishes the observed document and focus;
        // no synthetic classifier result or injected Input implementation.
        fixture
            .input(json!({"kind":"click","x":30,"y":25}))
            .unwrap();
        fixture
    }
    fn request(&self, params: Value) -> Result<Value, String> {
        self.host.protected_request(
            "owner",
            Some("first"),
            "host.browser",
            params,
            json!({"values":[],"targets":[],"unknown":false}),
        )
    }
    fn bound(&self, op: &str) -> Value {
        json!({"op":op,"tab_id":self.tab["tab_id"],"generation":self.tab["generation"],"document_id":self.tab["document_id"]})
    }
    fn input(&self, input: Value) -> Result<Value, String> {
        let mut params = self.bound("input");
        params["input"] = input;
        self.request(params)
    }
    fn status(&self) -> String {
        let snapshot = self.request(self.bound("snapshot")).unwrap();
        snapshot["snapshot"]["accessibility_nodes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|node| {
                node["role"] == "StaticText"
                    && node["name"]
                        .as_str()
                        .is_some_and(|name| name.starts_with("submitted="))
            })
            .unwrap()["name"]
            .as_str()
            .unwrap()
            .to_string()
    }
}

impl Drop for NativeInput {
    fn drop(&mut self) {
        let _ = self.host.shutdown();
        self.stop.store(true, Ordering::Release);
        if let Some(server) = self.server.take() {
            server.join().unwrap();
        }
    }
}

#[test]
#[ignore = "MP-10: sandbox-capable native Chromium and disposable CHARIOX_MDACCESS_DRILL_ROOT required"]
fn mdaccess_native_retained_enter_cannot_submit_payment() {
    let fixture = NativeInput::new(
        r#"<form><input id="field" type="text" style="height:30px"><button id="pay" type="submit">Pay</button></form>"#,
    );
    fixture.host.set_focus("owner", Some("second"));
    let result = fixture.input(json!({"kind":"key","key":"Enter"}));
    assert!(
        fixture.status().starts_with("submitted=0 "),
        "MP-11: retained Enter dispatched a native payment submit"
    );
    assert!(result.unwrap_err().contains("requires focus"));
}

#[test]
#[ignore = "MP-10: sandbox-capable native Chromium and disposable CHARIOX_MDACCESS_DRILL_ROOT required"]
fn mdaccess_native_focused_tab_releases_without_stopping_browser() {
    let fixture = NativeInput::new(
        r#"<input id="field" type="text" style="height:30px"><button id="pay">Approve payment</button>"#,
    );
    let stream = fixture.request(fixture.bound("subscribe")).unwrap();
    fixture
        .input(json!({"kind":"key","key":"Tab"}))
        .expect("MP-08: focused Tab onto approval must complete both events");
    assert_eq!(
        fixture.status(),
        "submitted=0 releases=1 focus=pay",
        "MP-08: Tab must release on the approval button without activating it"
    );
    let state = fixture.request(json!({"op":"state"})).unwrap();
    assert_eq!(
        state["generation"], fixture.tab["generation"],
        "MP-08: navigation stopped Chromium"
    );
    assert_eq!(state["tabs"][0]["document_id"], fixture.tab["document_id"]);
    fixture.request(json!({"op":"poll","subscription_id":stream["subscription_id"],"generation":stream["generation"]})).expect("MP-08: idle stream must remain valid");
}

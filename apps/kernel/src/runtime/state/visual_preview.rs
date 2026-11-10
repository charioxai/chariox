//! `chariox.visual.preview`: a page rendered by a headless Chromium under the policy the conversation draws it with.
use super::visual::MAX_PAGE_BYTES;
use super::*;
use crate::runtime::bounded_process::AsyncBoundedCommandExt as _;

/// What the conversation allows a page (it mirrors the pane's frame policy): no data fetches, no frames, scripts only inline or
/// from these hosts.
const HOSTS: &str =
    "https://cdn.jsdelivr.net https://cdnjs.cloudflare.com https://unpkg.com https://esm.sh";
const HEIGHT_MARK: &str = "chariox-content-height=";
const WINDOW_HEIGHT: u32 = 1600;
const CONSOLE_LINES: usize = 40;
const CONSOLE_LINE_CHARS: usize = 300;

fn policy() -> String {
    format!("default-src 'none'; connect-src 'none'; frame-src 'none'; child-src 'none'; worker-src blob:; object-src 'none'; base-uri 'none'; form-action 'none'; img-src data: blob:; media-src data: blob:; font-src data: {HOSTS}; style-src 'unsafe-inline' {HOSTS}; script-src 'unsafe-inline' {HOSTS}")
}

fn refusal(message: impl Into<String>) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "visual.preview",
        message: message.into(),
    }
}

/// Where the kernel browser installs its pinned Chromium (`kernel-browser-linux.mjs`).
const PINNED_LINUX_CHROMIUM: &str =
    "/usr/lib/chariox/browser/chromium/155.0.8059.79/chrome-linux64/chrome";
/// The Chrome apps the kernel browser looks for under `/Applications` and `~/Applications`, in its order (`kernel-browser-macos.mjs`).
const MACOS_APPS: [&str; 3] = [
    "Chromium.app/Contents/MacOS/Chromium",
    "Google Chrome.app/Contents/MacOS/Google Chrome",
    "Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing",
];

/// The browser that renders the preview: the one the kernel browser is pinned to, else the one it would find on this platform.
fn browser() -> Result<std::path::PathBuf, DaemonError> {
    resolve_browser(
        std::env::var_os("CHARIOX_KERNEL_BROWSER_EXECUTABLE").filter(|v| !v.is_empty()),
        std::env::consts::OS,
        std::env::var_os("HOME").map(std::path::PathBuf::from),
        &std::env::var_os("PATH").unwrap_or_default(),
        |candidate| candidate.is_file(),
    )
}

fn resolve_browser(
    pinned: Option<std::ffi::OsString>,
    os: &str,
    home: Option<std::path::PathBuf>,
    path: &std::ffi::OsStr,
    is_file: impl Fn(&std::path::Path) -> bool,
) -> Result<std::path::PathBuf, DaemonError> {
    if let Some(pinned) = pinned {
        return Ok(pinned.into());
    }
    let candidates: Vec<std::path::PathBuf> = if os == "macos" {
        let roots = std::iter::once(std::path::PathBuf::from("/Applications")).chain(
            home.filter(|home| home.is_absolute())
                .map(|home| home.join("Applications")),
        );
        roots
            .flat_map(|root| MACOS_APPS.map(|app| root.join(app)))
            .collect()
    } else {
        let named = ["google-chrome", "chromium", "chromium-browser", "chrome"]
            .into_iter()
            .flat_map(|name| std::env::split_paths(path).map(move |dir| dir.join(name)));
        std::iter::once(PINNED_LINUX_CHROMIUM.into())
            .chain(named)
            .collect()
    };
    candidates
        .into_iter()
        .find(|candidate| is_file(candidate))
        .ok_or_else(|| refusal("this machine has no Chromium or Chrome to render with; check the page with the browser tools instead"))
}

/// The page with the conversation's policy first in its head, and a script that reports its height to the console.
fn framed(page: &str) -> String {
    let head = format!(
        r#"<meta charset="utf-8"><meta http-equiv="Content-Security-Policy" content="{}">"#,
        policy()
    );
    // The pane's own measure: the page's content, not the frame it sits in.
    let report = format!(
        r#"<script>addEventListener("load",function(){{setTimeout(function(){{var r=document.documentElement;console.log("{HEIGHT_MARK}"+Math.ceil(r.scrollHeight>r.clientHeight?r.scrollHeight:r.getBoundingClientRect().height)+"/"+innerHeight)}},400)}})</script>"#
    );
    let lower = page.to_ascii_lowercase();
    let after_tag = |tag: &str| {
        let start = lower.find(tag)?;
        Some(start + lower[start..].find('>')? + 1)
    };
    let document = if let Some(at) = after_tag("<head") {
        format!("{}{head}{}", &page[..at], &page[at..])
    } else if let Some(at) = after_tag("<html") {
        format!("{}<head>{head}</head>{}", &page[..at], &page[at..])
    } else {
        format!("<!doctype html><html><head>{head}</head><body>{page}</body></html>")
    };
    format!("{document}{report}")
}

/// A console line of Chromium's log, `[time:LEVEL:CONSOLE(12)] "message", source: ...`, as its message.
fn console_message(line: &str) -> Option<String> {
    let (_, rest) = line.split_once(":CONSOLE")?;
    let (_, rest) = rest.split_once("] ")?;
    let message = rest
        .rsplit_once(", source: ")
        .map_or(rest, |(message, _)| message);
    let message = message.trim().trim_matches('"');
    Some(message.chars().take(CONSOLE_LINE_CHARS).collect())
}

/// A directory of this call alone: `create_dir` fails when the name is taken, so two calls never share one.
fn create_work_dir(parent: &std::path::Path) -> std::io::Result<std::path::PathBuf> {
    static CALLS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    std::fs::create_dir_all(parent)?;
    loop {
        let call = CALLS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let work = parent.join(format!(
            "preview-{}-{}-{call}",
            crate::session::unix_epoch_ms(),
            std::process::id()
        ));
        match std::fs::create_dir(&work) {
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            created => return created.map(|()| work),
        }
    }
}

pub(super) async fn preview(page: &str, width: u32) -> Result<serde_json::Value, DaemonError> {
    if !(320..=1600).contains(&width) {
        return Err(refusal("width must be 320-1600"));
    }
    let browser = browser()?;
    let work = create_work_dir(&crate::config::default_state_dir().join("visuals"))
        .map_err(|error| refusal(format!("cannot render: {error}")))?;
    let rendered = render(&browser, &work, page, width).await;
    let _ = std::fs::remove_dir_all(&work);
    let (png, log) = rendered?;
    let mut measured = None;
    let mut console = Vec::new();
    for message in log.lines().filter_map(console_message) {
        match message
            .strip_prefix(HEIGHT_MARK)
            .and_then(|sizes| sizes.split_once('/'))
        {
            Some((height, viewport)) => {
                measured = height.parse::<u32>().ok().zip(viewport.parse::<u32>().ok())
            }
            None if console.len() < CONSOLE_LINES => console.push(message),
            None => {}
        }
    }
    Ok(serde_json::json!({
        "mime_type": "image/png",
        "image_base64": base64::engine::general_purpose::STANDARD.encode(&png),
        "width": width,
        "content_height": measured.map(|(height, _)| height),
        // A page taller than the window is cut off in the picture.
        "clipped": measured.is_some_and(|(height, viewport)| height > viewport),
        "console": console,
    }))
}

async fn render(
    browser: &std::path::Path,
    work: &std::path::Path,
    page: &str,
    width: u32,
) -> Result<(Vec<u8>, String), DaemonError> {
    if page.len() as u64 > MAX_PAGE_BYTES {
        return Err(refusal("the page is larger than 1 MB"));
    }
    let file = work.join("page.html");
    let picture = work.join("page.png");
    std::fs::write(&file, framed(page))
        .map_err(|error| refusal(format!("cannot render: {error}")))?;
    let mut command = crate::process_spawn::async_command(browser);
    command
        // Chromium puts a socket under TMPDIR, and a long TMPDIR makes the path too long for it.
        .env("TMPDIR", "/tmp")
        .args([
            "--headless=new",
            "--disable-gpu",
            "--hide-scrollbars",
            "--no-first-run",
            "--disable-extensions",
            "--force-device-scale-factor=1",
            "--virtual-time-budget=5000",
            "--enable-logging=stderr",
            "--v=0",
        ])
        .arg(format!("--window-size={width},{WINDOW_HEIGHT}"))
        .arg(format!(
            "--user-data-dir={}",
            work.join("profile").display()
        ))
        .arg(format!("--screenshot={}", picture.display()));
    // Chromium refuses to start as root with its sandbox; the page is the agent's own work on the agent's own machine.
    if unsafe { libc::geteuid() } == 0 {
        command.arg("--no-sandbox");
    }
    let output = command
        .arg(format!("file://{}", file.display()))
        .bounded_output()
        .await
        .map_err(|error| refusal(format!("the browser did not render the page: {error}")))?;
    let png = std::fs::read(&picture)
        .map_err(|_| refusal("the browser produced no picture of the page"))?;
    Ok((png, String::from_utf8_lossy(&output.stderr).into_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_page_gets_the_conversations_policy_in_its_head() {
        let html =
            framed("<!doctype html><html><head><title>t</title></head><body>x</body></html>");
        let head = html.find("<head>").unwrap();
        assert!(html[head..].starts_with("<head><meta charset"));
        assert!(html.contains("Content-Security-Policy"));
        assert!(html.contains("<!doctype html>") && html.find("<!doctype").unwrap() < head);
        let bare = framed("<div>x</div>");
        assert!(bare.starts_with("<!doctype html><html><head><meta"));
    }

    #[test]
    fn console_lines_come_back_as_their_message() {
        let line =
            r#"[1110/101010.123:INFO:CONSOLE(12)] "chart ready", source: file:///x/page.html (12)"#;
        assert_eq!(console_message(line).as_deref(), Some("chart ready"));
        assert_eq!(console_message("[1110/1:ERROR:gpu_init.cc(1)] nope"), None);
    }

    fn found(os: &str, home: Option<&str>, path: &str, files: &[&str]) -> Option<String> {
        resolve_browser(
            None,
            os,
            home.map(Into::into),
            std::ffi::OsStr::new(path),
            |candidate| {
                files
                    .iter()
                    .any(|file| candidate == std::path::Path::new(file))
            },
        )
        .ok()
        .map(|found| found.display().to_string())
    }

    const MAC_CHROME: &str = "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";

    #[test]
    fn a_mac_renders_with_the_chrome_in_applications() {
        assert_eq!(
            found("macos", Some("/Users/a"), "/usr/bin", &[MAC_CHROME]).as_deref(),
            Some(MAC_CHROME)
        );
    }

    #[test]
    fn a_mac_renders_with_a_chrome_in_the_users_applications() {
        let user = "/Users/a/Applications/Chromium.app/Contents/MacOS/Chromium";
        assert_eq!(
            found("macos", Some("/Users/a"), "", &[user]).as_deref(),
            Some(user)
        );
    }

    #[test]
    fn a_mac_prefers_the_system_applications_and_chromium_like_the_kernel_browser() {
        let chromium = "/Applications/Chromium.app/Contents/MacOS/Chromium";
        let user = "/Users/a/Applications/Chromium.app/Contents/MacOS/Chromium";
        assert_eq!(
            found("macos", Some("/Users/a"), "", &[MAC_CHROME, chromium, user]).as_deref(),
            Some(chromium)
        );
        assert_eq!(
            found("macos", Some("/Users/a"), "", &[MAC_CHROME, user]).as_deref(),
            Some(MAC_CHROME)
        );
    }

    #[test]
    fn a_linux_machine_renders_with_a_chromium_on_the_path_or_the_pinned_one() {
        assert_eq!(
            found("linux", None, "/opt/bin:/usr/bin", &["/usr/bin/chromium"]).as_deref(),
            Some("/usr/bin/chromium")
        );
        let pinned = "/usr/lib/chariox/browser/chromium/155.0.8059.79/chrome-linux64/chrome";
        assert_eq!(
            found("linux", None, "/usr/bin", &[pinned]).as_deref(),
            Some(pinned)
        );
    }

    #[test]
    fn the_pinned_browser_wins_and_a_machine_without_one_says_so() {
        let pinned = resolve_browser(
            Some("/opt/chrome".into()),
            "macos",
            None,
            std::ffi::OsStr::new(""),
            |_| false,
        );
        assert_eq!(pinned.unwrap(), std::path::PathBuf::from("/opt/chrome"));
        assert_eq!(found("macos", Some("/Users/a"), "/usr/bin", &[]), None);
    }

    #[test]
    fn the_browser_lists_follow_the_kernel_browsers() {
        let macos = include_str!("../../../slice-linux-docker/docker/kernel-browser-macos.mjs");
        for app in MACOS_APPS {
            assert!(
                macos.contains(app),
                "{app} is not a macOS candidate of the kernel browser"
            );
        }
        let linux = include_str!("../../../slice-linux-docker/docker/kernel-browser-linux.mjs");
        assert!(linux.contains(PINNED_LINUX_CHROMIUM));
    }

    const FAKE_BROWSER: &str = r#"#!/bin/sh
for argument in "$@"; do
  case "$argument" in
    --screenshot=*) picture="${argument#--screenshot=}" ;;
    file://*) page="${argument#file://}" ;;
  esac
done
sleep 0.3
cp "$page" "$picture"
"#;

    #[tokio::test]
    async fn previews_started_together_each_render_their_own_page() {
        use std::os::unix::fs::PermissionsExt as _;
        let _environment = crate::env_lock::lock();
        let root = std::env::temp_dir().join(format!(
            "visual-preview-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let browser = root.join("browser.sh");
        std::fs::write(&browser, FAKE_BROWSER).unwrap();
        std::fs::set_permissions(&browser, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::env::set_var("CHARIOX_HOME", root.join("home"));
        std::env::set_var("CHARIOX_KERNEL_BROWSER_EXECUTABLE", &browser);
        let previews = (0..4).map(|n| async move {
            let shown = preview(&format!("<p>page-number-{n}</p>"), 800).await;
            (n, shown)
        });
        let results = futures_util::future::join_all(previews).await;
        let _ = std::fs::remove_dir_all(&root);
        for (n, shown) in results {
            let shown = shown.unwrap_or_else(|error| panic!("preview {n} failed: {error:?}"));
            let png = base64::engine::general_purpose::STANDARD
                .decode(shown["image_base64"].as_str().unwrap())
                .unwrap();
            let rendered = String::from_utf8(png).unwrap();
            assert!(
                rendered.contains(&format!("page-number-{n}<")),
                "preview {n} got another page: {rendered}"
            );
        }
    }
}

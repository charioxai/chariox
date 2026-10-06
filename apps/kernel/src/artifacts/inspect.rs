//! MP-08/MP-10/MP-11: bounded PDF extraction from an admitted operational blob.
use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::time::Duration;
use wait_timeout::ChildExt;

pub(crate) fn pdf_text(input: &[u8]) -> Result<String, String> {
    let mut command = Command::new("pdftotext");
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // MP-08/MP-10/MP-11: hostile PDFs also have a child memory/CPU bound.
        unsafe {
            command.pre_exec(|| {
                for (resource, limit) in
                    [(libc::RLIMIT_AS, 256 * 1024 * 1024), (libc::RLIMIT_CPU, 3)]
                {
                    let limit = libc::rlimit {
                        rlim_cur: limit,
                        rlim_max: limit,
                    };
                    if libc::setrlimit(resource, &limit) != 0 {
                        return Err(std::io::Error::last_os_error());
                    }
                }
                Ok(())
            });
        }
    }
    let mut child = command
        .args(["-layout", "-enc", "UTF-8", "-nopgbrk"])
        .arg("-")
        .arg("-")
        .stdin(Stdio::piped())
        .stderr(Stdio::null())
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|_| "PDF inspection requires the installed pdftotext utility")?;
    let mut stdin = child.stdin.take().unwrap();
    let input = input.to_vec();
    let writer = std::thread::spawn(move || stdin.write_all(&input));
    let stdout = child.stdout.take().unwrap();
    let reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        stdout.take(262145).read_to_end(&mut bytes).map(|_| bytes)
    });
    let status = child.wait_timeout(Duration::from_secs(3));
    if !matches!(status, Ok(Some(_))) {
        let _ = child.kill();
        let _ = child.wait();
    }
    let _ = writer.join();
    let bytes = reader
        .join()
        .map_err(|_| "PDF inspection reader failed")?
        .map_err(|_| "PDF inspection read failed")?;
    if !matches!(status, Ok(Some(status)) if status.success()) || bytes.len() > 262144 {
        return Err("PDF inspection failed or exceeded its time/output bound".into());
    }
    String::from_utf8(bytes).map_err(|_| "PDF inspection returned invalid UTF-8".into())
}

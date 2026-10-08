use super::*;

mod seatbelt_tests {
    use super::*;

    #[test]
    fn compiler_isolation_denies_host_files_commands_environment_and_network() {
        let node = discover_workflow_code_node_path().expect("real Node required");
        let mut command = compiler_command(&node, &WorkflowCodeLimitsConfig::default()).unwrap();
        // Trusted diagnostics test the OS boundary directly. These imports are
        // never provided to workflow source or to an agent prompt.
        let diagnostic = r#"
if (Object.keys(process.env).length !== 0) throw new Error('inherited environment');
import fs from 'node:fs';
import net from 'node:net';
import {spawnSync} from 'node:child_process';
if (process.cwd() !== '/tmp' || fs.readdirSync('/tmp').length) throw new Error('scratch not empty');
let readonly = false;
try {fs.writeFileSync('/marker', 'x')} catch (error) {readonly = ['EROFS', 'EPERM', 'EACCES'].includes(error.code)}
if (!readonly) throw new Error('root is writable');
fs.writeFileSync('/tmp/marker', 'x'); fs.unlinkSync('/tmp/marker');
for (const path of ['/root', '/etc/hostname', '/bin/sh', '/proc']) {
  let readable = false; try { fs.readFileSync(path); readable = true; } catch {}
  if (readable) throw new Error('host file exposed');
}
const child = spawnSync('/bin/sh', ['-c', 'exit 0']);
if (!['ENOENT', 'EPERM', 'EACCES'].includes(child.error?.code)) throw new Error('host command available');
{
const child = spawnSync(process.execPath, ["--version"]);
if (!['ENOENT', 'EPERM', 'EACCES'].includes(child.error?.code)) throw new Error('host command available');
}
if (!['ENOENT', 'EPERM', 'EACCES'].includes(child.error?.code)) throw new Error('host command available');
await new Promise((resolve, reject) => {
 const socket = net.connect({host: '1.1.1.1', port: 443});
 socket.on('connect', () => {socket.destroy(); reject(new Error('network available'))});
 socket.on('error', () => resolve());
 socket.setTimeout(1000, () => {socket.destroy(); resolve()});
});
process.stdout.write('isolated');
"#;
        let diagnostic =
            diagnostic.replace("/tmp", command.get_current_dir().unwrap().to_str().unwrap());
        let output = command
            .args([
                "--disable-wasm-trap-handler",
                "--input-type=module",
                "-e",
                &diagnostic,
            ])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(output.stdout, b"isolated");
    }

    #[test]
    fn compiler_isolation_does_not_inherit_parent_file_handles() {
        use std::os::fd::{AsRawFd, FromRawFd};
        let worktree = crate::test_support::TestWorktree::new("compiler-parent-handle");
        let path = worktree.path().join("public-fixture.txt");
        fs::write(&path, "MP-08 / MP-11 public fixture").unwrap();
        let file = fs::File::open(path).unwrap();
        let descriptor = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_DUPFD, 128) };
        assert!(descriptor >= 128);
        let _owned_handle = unsafe { fs::File::from_raw_fd(descriptor) };
        let node = discover_workflow_code_node_path().unwrap();
        let mut command = compiler_command(&node, &WorkflowCodeLimitsConfig::default()).unwrap();
        let diagnostic = format!("const fs=require('node:fs');try{{fs.fstatSync({descriptor});process.exit(1)}}catch(e){{if(e.code!=='EBADF')process.exit(2)}}");
        let output = command
            .args(["--disable-wasm-trap-handler", "-e", &diagnostic])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn compiler_isolation_schema_open_rejects_parent_directory_links() {
        use std::os::unix::fs::symlink;
        let root = crate::test_support::TestWorktree::new("compiler-schema-root");
        let outside = crate::test_support::TestWorktree::new("compiler-schema-outside");
        fs::write(outside.path().join("sample.json"), r#"{"type":"string"}"#).unwrap();
        symlink(outside.path(), root.path().join("schemas")).unwrap();
        let directory = fs::File::open(root.path()).unwrap();
        assert!(open_schema_file(
            &directory,
            root.path(),
            &root.path().join("schemas/sample.json")
        )
        .is_err());
        assert!(open_schema_root(&root.path().join("schemas")).is_err());
    }

    #[test]
    fn compiler_isolation_schema_open_rejects_nonregular_files_without_waiting() {
        use std::os::unix::ffi::OsStrExt;
        let root = crate::test_support::TestWorktree::new("compiler-schema-pipe");
        let path = root.path().join("schema.json");
        let name = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
        let directory = fs::File::open(root.path()).unwrap();
        let error = open_schema_file(&directory, root.path(), &path).unwrap_err();
        assert!(error.to_string().contains("regular file"));
    }

    #[test]
    fn compiler_isolation_enforces_async_timeout() {
        let limits = WorkflowCodeLimitsConfig {
            script_timeout_ms: 500,
            ..Default::default()
        };
        let error =
            compile_workflow_code_javascript("ignored", "await new Promise(() => {})", &limits)
                .unwrap_err();
        // Node may settle an unresolved top-level await itself or the parent
        // timeout may reap it; neither can hold the kernel indefinitely.
        assert!(
            error.to_string().contains("compiler failed") || error.to_string().contains("timeout"),
            "{error}"
        );
    }
}

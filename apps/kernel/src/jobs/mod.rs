//! Source snapshots and conditional file returns for kernel-owned jobs.
mod workspace;
pub(crate) use workspace::*;

/// The selected environment arrives over a private binary pipe, never argv or a file.
pub(crate) const COMMAND_WRAPPER: &str = r#"import json,os,sys
request=json.loads(sys.stdin.buffer.readline())
env={key:os.environ[key] for key in ('PATH','HOME','USER','LOGNAME','LANG','LC_ALL','LC_CTYPE','TZ','TMPDIR') if key in os.environ}
env.update(request['env'])
env['CHARIOX_AGENT_WAKE_ID']=request['job_id']
os.execvpe(request['argv'][0],request['argv'],env)
"#;

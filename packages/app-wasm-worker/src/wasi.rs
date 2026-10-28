//! R1's enumerated WASI surface, with no ambient network or filesystem.
use crate::{attribution::Attribution, host::Host, ipc::Ipc};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
use wasmtime::component::{Linker, ResourceTable};
use wasmtime_wasi::{filesystem::FsPerms, WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};
use wasmtime_wasi_http::{WasiHttpCtx, WasiHttpCtxView, WasiHttpView};

mod http;
mod options;
mod output;

/// The supervisor supplies the private installation root after establishing
/// its generation fence and OS containment. Guest input never selects it.
#[derive(Clone, Debug)]
pub struct InstallationDirectories(PathBuf);
impl InstallationDirectories {
    pub fn open(root: &Path) -> wasmtime::Result<Self> {
        wasmtime::ensure!(
            root.is_absolute(),
            "APP_PREOPEN_INVALID: installation root must be absolute"
        );
        let root = root.canonicalize()?;
        for name in ["data", "tmp"] {
            wasmtime::ensure!(
                std::fs::symlink_metadata(root.join(name))?.is_dir(),
                "APP_PREOPEN_INVALID: installation {name} must be a real directory"
            );
        }
        Ok(Self(root))
    }
    fn configure(&self, builder: &mut WasiCtxBuilder) -> wasmtime::Result<()> {
        // Recheck at restart; never silently replace a missing installation.
        Self::open(&self.0)?;
        for name in ["data", "tmp"] {
            builder.preopened_dir(self.0.join(name), name, FsPerms::ReadWrite)?;
        }
        Ok(())
    }
}

pub struct Context {
    ctx: WasiCtx,
    table: ResourceTable,
    http: WasiHttpCtx,
    hooks: http::Broker,
}
impl Context {
    pub(crate) fn new(
        ipc: Ipc,
        attribution: Attribution,
        slots: Arc<tokio::sync::Semaphore>,
        directories: Option<&InstallationDirectories>,
    ) -> wasmtime::Result<Self> {
        let mut builder = WasiCtxBuilder::new();
        builder.stdout(output::Output::new(ipc.clone(), "info"));
        builder.stderr(output::Output::new(ipc.clone(), "error"));
        if let Some(directories) = directories {
            directories.configure(&mut builder)?;
        }
        Ok(Self {
            ctx: builder.build(),
            table: ResourceTable::new(),
            http: WasiHttpCtx::new(),
            hooks: http::Broker::new(ipc, attribution, slots),
        })
    }
}
impl WasiView for Host {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi.ctx,
            table: &mut self.wasi.table,
        }
    }
}
impl WasiHttpView for Host {
    fn http(&mut self) -> WasiHttpCtxView<'_> {
        WasiHttpCtxView {
            ctx: &mut self.wasi.http,
            table: &mut self.wasi.table,
            hooks: &mut self.wasi.hooks,
        }
    }
}

pub(crate) fn add_to_linker(linker: &mut Linker<Host>) -> wasmtime::Result<()> {
    use wasmtime::component::HasSelf;
    use wasmtime_wasi::random::WasiRandomView;
    use wasmtime_wasi::{
        cli::{WasiCli, WasiCliView},
        clocks::{WasiClocks, WasiClocksView},
        filesystem::{WasiFilesystem, WasiFilesystemView},
        random::WasiRandom,
    };
    use wasmtime_wasi::{p2::bindings as p2, p3::bindings as p3};
    use wasmtime_wasi_io::bindings::wasi::io;
    // Deliberately enumerate. The upstream all-interfaces helpers also link
    // sockets, environment, process exit and terminals, which R1 forbids.
    io::error::add_to_linker::<_, HasSelf<ResourceTable>>(linker, |h| &mut h.wasi.table)?;
    io::poll::add_to_linker::<_, HasSelf<ResourceTable>>(linker, |h| &mut h.wasi.table)?;
    io::streams::add_to_linker::<_, HasSelf<ResourceTable>>(linker, |h| &mut h.wasi.table)?;
    p2::clocks::monotonic_clock::add_to_linker::<_, WasiClocks>(linker, Host::clocks)?;
    p2::clocks::wall_clock::add_to_linker::<_, WasiClocks>(linker, Host::clocks)?;
    p2::random::random::add_to_linker::<_, WasiRandom>(linker, Host::random)?;
    p2::filesystem::types::add_to_linker::<_, WasiFilesystem>(linker, Host::filesystem)?;
    p2::filesystem::preopens::add_to_linker::<_, WasiFilesystem>(linker, Host::filesystem)?;
    p2::cli::stdout::add_to_linker::<_, WasiCli>(linker, Host::cli)?;
    p2::cli::stderr::add_to_linker::<_, WasiCli>(linker, Host::cli)?;
    wasmtime_wasi_http::p2::add_only_http_to_linker_async(linker)?;
    wasmtime_wasi::p3::clocks::add_to_linker(linker)?;
    p3::random::random::add_to_linker::<_, WasiRandom>(linker, Host::random)?;
    wasmtime_wasi::p3::filesystem::add_to_linker(linker)?;
    p3::cli::stdout::add_to_linker::<_, WasiCli>(linker, Host::cli)?;
    p3::cli::stderr::add_to_linker::<_, WasiCli>(linker, Host::cli)?;
    wasmtime_wasi_http::p3::add_to_linker(linker)?;
    options::add_to_linker(linker)?;
    Ok(())
}

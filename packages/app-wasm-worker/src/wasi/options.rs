//! The broker does not expose TCP/TLS phase timing. Refuse that optional WASI
//! setting at its setter rather than pretending stream admission is a connect.
use crate::host::Host;
use wasmtime::{
    component::{Linker, Resource},
    StoreContextMut,
};
use wasmtime_wasi_http::{
    p3::{bindings::http::types::RequestOptionsError, MaybeMutable},
    RequestOptions, WasiHttpView,
};

pub(super) fn add_to_linker(linker: &mut Linker<Host>) -> wasmtime::Result<()> {
    // These replace only setters in the pinned upstream interfaces. Keep every
    // other HTTP resource/method and restore the default no-shadowing policy.
    linker.allow_shadowing(true);
    linker.instance("wasi:http/types@0.2.12")?.func_wrap(
        "[method]request-options.set-connect-timeout",
        |mut store: StoreContextMut<'_, Host>,
         (options, duration): (Resource<RequestOptions>, Option<u64>)|
         -> wasmtime::Result<(Result<(), ()>,)> {
            let view = store.data_mut().http();
            let options = view.table.get_mut(&options)?;
            if duration.is_some() {
                return Ok((Err(()),));
            }
            options.connect_timeout = None;
            Ok((Ok(()),))
        },
    )?;
    linker.instance("wasi:http/types@0.3.0")?.func_wrap(
        "[method]request-options.set-connect-timeout",
        |mut store: StoreContextMut<'_, Host>,
         (options, duration): (Resource<MaybeMutable<RequestOptions>>, Option<u64>)|
         -> wasmtime::Result<(Result<(), RequestOptionsError>,)> {
            let view = store.data_mut().http();
            let options = view.table.get_mut(&options)?;
            let Some(options) = options.get_mut() else {
                return Ok((Err(RequestOptionsError::Immutable),));
            };
            if duration.is_some() {
                return Ok((Err(RequestOptionsError::NotSupported),));
            }
            options.connect_timeout = None;
            Ok((Ok(()),))
        },
    )?;
    linker.allow_shadowing(false);
    Ok(())
}

//! Hardened-runtime code memory using Wasmtime's public raw deserializer.
//! Only compiler output enters MAP_JIT; guest memories use Wasmtime's allocator.
use std::{
    ffi::c_void,
    io,
    ptr::NonNull,
    sync::{Arc, Mutex},
};
use wasmtime::{component::Component, CustomCodeMemory, Engine, Module};

unsafe extern "C" {
    fn pthread_jit_write_protect_np(enabled: i32);
    fn sys_icache_invalidate(start: *mut c_void, length: usize);
}

struct Mapping {
    address: usize,
    length: usize,
}
impl Drop for Mapping {
    fn drop(&mut self) {
        unsafe { libc::munmap(self.address as *mut c_void, self.length) };
    }
}

pub(crate) struct JitCode {
    page_size: usize,
    // Wasmtime's CodeMemory and Engine retain this publisher through Arc.
    // Keeping allocations here outlives raw Components/Modules, their clones,
    // instances, and unwind registrations, including after WorkerEngine drops.
    mappings: Mutex<Vec<Mapping>>,
}

pub(crate) fn engine() -> wasmtime::Result<(Engine, Arc<JitCode>)> {
    let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    wasmtime::ensure!(page_size > 0, "JIT page size unavailable");
    let page_size = page_size as usize;
    wasmtime::ensure!(page_size.is_power_of_two(), "invalid JIT page size");
    let code = Arc::new(JitCode {
        page_size,
        mappings: Mutex::new(Vec::new()),
    });
    let mut config = crate::engine::configuration();
    config.with_custom_code_memory(Some(code.clone()));
    Ok((Engine::new(&config)?, code))
}

impl JitCode {
    fn allocate(&self, bytes: &[u8]) -> wasmtime::Result<NonNull<[u8]>> {
        wasmtime::ensure!(!bytes.is_empty(), "empty compiled artifact");
        let length = bytes
            .len()
            .checked_add(self.page_size - 1)
            .ok_or_else(|| wasmtime::Error::msg("compiled artifact too large"))?
            & !(self.page_size - 1);
        let address = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                length,
                libc::PROT_READ | libc::PROT_WRITE | libc::PROT_EXEC,
                libc::MAP_PRIVATE | libc::MAP_ANON | libc::MAP_JIT,
                -1,
                0,
            )
        };
        if address == libc::MAP_FAILED {
            return Err(wasmtime::Error::msg(format!(
                "mmap(MAP_JIT): {}",
                io::Error::last_os_error()
            )));
        }
        let mapping = Mapping {
            address: address as usize,
            length,
        };
        // Apple Silicon enforces W^X per thread for MAP_JIT. No guest code is
        // running on this compiler thread while its write mode is enabled.
        unsafe {
            pthread_jit_write_protect_np(0);
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), address.cast::<u8>(), bytes.len());
            pthread_jit_write_protect_np(1);
        }
        // Apple Silicon MAP_JIT mappings keep RWX VM permissions and enforce
        // W^X through the thread's JIT mode. mprotect, even to R, is refused.
        #[cfg(not(target_arch = "aarch64"))]
        protect(address.cast(), length, libc::PROT_READ)?;
        self.mappings
            .lock()
            .map_err(|_| wasmtime::Error::msg("JIT allocation ownership poisoned"))?
            .push(mapping);
        Ok(NonNull::slice_from_raw_parts(
            NonNull::new(address.cast()).expect("mmap returned non-null code"),
            bytes.len(),
        ))
    }

    pub(crate) unsafe fn component(
        &self,
        engine: &Engine,
        bytes: &[u8],
    ) -> wasmtime::Result<Component> {
        unsafe { Component::deserialize_raw(engine, self.allocate(bytes)?) }
    }

    pub(crate) unsafe fn module(&self, engine: &Engine, bytes: &[u8]) -> wasmtime::Result<Module> {
        unsafe { Module::deserialize_raw(engine, self.allocate(bytes)?) }
    }
}

impl CustomCodeMemory for JitCode {
    fn required_alignment(&self) -> usize {
        self.page_size
    }

    fn publish_executable(&self, ptr: *const u8, len: usize) -> wasmtime::Result<()> {
        let address = ptr as usize;
        let mappings = self
            .mappings
            .lock()
            .map_err(|_| wasmtime::Error::msg("JIT allocation ownership poisoned"))?;
        wasmtime::ensure!(
            mappings.iter().any(|m| address >= m.address
                && address
                    .checked_add(len)
                    .is_some_and(|end| end <= m.address + m.length)),
            "executable code is outside owned MAP_JIT memory"
        );
        #[cfg(not(target_arch = "aarch64"))]
        protect(ptr, len, libc::PROT_READ | libc::PROT_EXEC)?;
        // The raw image is immutable for its whole Wasmtime lifetime. On
        // Apple Silicon its metadata shares the JIT region's execute mode;
        // it is never used as writable guest data or exposed to guest code.
        unsafe {
            pthread_jit_write_protect_np(1);
            sys_icache_invalidate(ptr.cast_mut().cast(), len);
        }
        Ok(())
    }

    fn unpublish_executable(&self, ptr: *const u8, len: usize) -> wasmtime::Result<()> {
        #[cfg(not(target_arch = "aarch64"))]
        protect(ptr, len, libc::PROT_READ | libc::PROT_WRITE)?;
        #[cfg(target_arch = "aarch64")]
        let _ = (ptr, len);
        // Do not put this thread in write mode while another module may still
        // execute on it. The immutable allocation is unmapped when the final
        // publisher/engine/code reference drops, after unwind deregistration.
        Ok(())
    }
}

#[cfg(not(target_arch = "aarch64"))]
fn protect(ptr: *const u8, len: usize, permissions: i32) -> wasmtime::Result<()> {
    if unsafe { libc::mprotect(ptr.cast_mut().cast(), len, permissions) } != 0 {
        return Err(wasmtime::Error::msg(format!(
            "mprotect(JIT code, permissions={permissions}): {}",
            io::Error::last_os_error()
        )));
    }
    Ok(())
}

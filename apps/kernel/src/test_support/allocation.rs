//! Counts what the current thread allocates, so a test can bound the work of a
//! scan or the memory a Room keeps instead of timing them.
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::future::Future;

struct Counting;

thread_local! {
    static ALLOCATED: Cell<usize> = const { Cell::new(0) };
    static LIVE: Cell<isize> = const { Cell::new(0) };
}

fn count(allocated: usize, live: isize) {
    // A thread that is being torn down has no counters; nothing reads them then.
    let _ = ALLOCATED.try_with(|total| total.set(total.get() + allocated));
    let _ = LIVE.try_with(|net| net.set(net.get() + live));
}

#[global_allocator]
static COUNTING: Counting = Counting;

// SAFETY: every call is forwarded unchanged to the system allocator.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        count(layout.size(), layout.size() as isize);
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        count(layout.size(), layout.size() as isize);
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        count(0, -(layout.size() as isize));
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        count(new_size, new_size as isize - layout.size() as isize);
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

/// Bytes allocated and bytes still held, on this thread, while `future` runs.
/// Run it on a current-thread runtime so the work stays on this thread.
pub(crate) async fn measured<F: Future>(future: F) -> (F::Output, Measured) {
    let before = (ALLOCATED.get(), LIVE.get());
    let output = future.await;
    let after = (ALLOCATED.get(), LIVE.get());
    (
        output,
        Measured {
            allocated: after.0 - before.0,
            retained: after.1 - before.1,
        },
    )
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Measured {
    pub(crate) allocated: usize,
    pub(crate) retained: isize,
}

/// A kernel whose store holds `rooms` Rooms, none of them using triggers,
/// publications or steers.
pub(crate) fn runtime_with_rooms(rooms: usize) -> crate::runtime::state::KernelRuntimeState {
    let mut app = crate::DaemonApp::bootstrap(crate::DaemonConfig::for_tests())
        .expect("the kernel should boot");
    let mut sessions = crate::app::KernelSessionService::new(&mut app);
    for _ in 0..rooms {
        sessions
            .create_session(crate::session::CreateSessionRequest::new(
                "workspace",
                "worktree",
            ))
            .expect("the Room should be created");
    }
    crate::runtime::router::CommandRouter::with_interactive_capacity(
        std::sync::Arc::new(tokio::sync::Mutex::new(app)),
        1,
    )
    .runtime_state()
}

//! The kernel's allocator policy for memory that does not go through mimalloc.

/// SQLite and other C code allocate through glibc, which gives every thread that calls it its own
/// 64 MiB arena and keeps the memory a thread frees. A kernel under load grew 5 to 6 MB/min in
/// those arenas; one arena keeps it flat at no measurable CPU cost.
#[cfg(all(target_os = "linux", target_env = "gnu"))]
pub(crate) fn configure() {
    // SAFETY: mallopt only changes glibc's arena limit, and runs before the kernel starts threads.
    unsafe { libc::mallopt(libc::M_ARENA_MAX, 1) };
}

#[cfg(not(all(target_os = "linux", target_env = "gnu")))]
pub(crate) fn configure() {}

#[cfg(all(test, target_os = "linux", target_env = "gnu"))]
mod tests {
    use std::process::Command;

    const MODE: &str = "CHARIOX_ALLOCATOR_TEST_MODE";
    const TEST: &str = "allocator::tests::threads_share_one_c_allocator_arena";
    const ARENA_HEAP: u64 = 64 * 1024 * 1024;

    /// glibc reserves 64 MiB (PROT_NONE) for the heap of every thread arena it creates, and
    /// maps the used part of the heap read-write in front of it.
    fn arena_reservations() -> usize {
        std::fs::read_to_string("/proc/self/maps")
            .unwrap()
            .lines()
            .filter(|line| {
                let mut fields = line.split_whitespace();
                let (range, permissions) = (fields.next().unwrap(), fields.next().unwrap());
                let (start, end) = range.split_once('-').unwrap();
                let size =
                    u64::from_str_radix(end, 16).unwrap() - u64::from_str_radix(start, 16).unwrap();
                permissions == "---p" && size > ARENA_HEAP - 1024 * 1024 && size < ARENA_HEAP
            })
            .count()
    }

    /// New reservations after four threads each made a C allocation.
    fn new_reservations_after_four_threads_allocate() -> usize {
        let before = arena_reservations();
        let allocations: Vec<_> = (0..4)
            .map(|_| std::thread::spawn(|| unsafe { libc::malloc(4096) as usize }))
            .map(|thread| thread.join().unwrap())
            .collect();
        let reserved = arena_reservations() - before;
        for allocation in allocations {
            unsafe { libc::free(allocation as *mut libc::c_void) };
        }
        reserved
    }

    fn run_in_own_process(mode: &str) -> usize {
        let output = Command::new("/proc/self/exe")
            .args([TEST, "--exact", "--nocapture", "--test-threads=1"])
            .env(MODE, mode)
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .find_map(|line| line.split("reservations=").nth(1))
            .expect("the child reports its reservations")
            .parse()
            .unwrap()
    }

    #[test]
    fn threads_share_one_c_allocator_arena() {
        // Arenas are per process, and the harness's own threads create some: count in a child.
        match std::env::var(MODE).as_deref() {
            Ok("default") => println!(
                "reservations={}",
                new_reservations_after_four_threads_allocate()
            ),
            Ok("configured") => {
                super::configure();
                println!(
                    "reservations={}",
                    new_reservations_after_four_threads_allocate()
                );
            }
            _ => {
                let default = run_in_own_process("default");
                assert!(default > 0, "glibc made no thread arenas to compare with");
                assert_eq!(run_in_own_process("configured"), 0);
            }
        }
    }
}

//! Run Chromium's SIGWINCH handler on the browser main thread.
//!
//! The Chromium side installs `signal(SIGWINCH, ...)` with a handler that
//! resizes the web contents directly. The kernel delivers a process signal
//! to any thread that does not block it, so that handler sometimes ran on
//! Chromium's sandbox IPC thread, whose thread check then stopped the whole
//! browser with SIGTRAP while a window was being resized. Work like that is
//! not allowed inside a signal handler in any case.
//!
//! `route_to_main_thread` replaces the handler with one that only wakes a
//! dispatcher thread through a pipe (async-signal-safe), and the dispatcher
//! posts Chromium's original handler to the main thread with `post_task`,
//! once per burst of resizes.

use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::time::Duration;

use libc::c_int;

/// Same signature as the `post_task` callback of the browser delegate.
pub type PostTask = extern "C" fn(extern "C" fn(*mut libc::c_void), *mut libc::c_void);

static WAKE_FD: AtomicI32 = AtomicI32::new(-1);
static PENDING: AtomicBool = AtomicBool::new(false);
/// Bursts closer together than this become one resize.
const COALESCE: Duration = Duration::from_millis(15);

extern "C" fn on_sigwinch(_: c_int) {
    // Only async-signal-safe calls here: an atomic swap and write(2).
    if !PENDING.swap(true, Ordering::SeqCst) {
        let fd = WAKE_FD.load(Ordering::SeqCst);

        if fd >= 0 {
            let byte = 1u8;
            unsafe { libc::write(fd, &byte as *const u8 as *const libc::c_void, 1) };
        }
    }
}

/// Take over SIGWINCH and forward it to the handler that was installed
/// before, run on the thread `post_task` posts to. Does nothing when no
/// handler function is installed. Returns whether it took over.
pub fn route_to_main_thread(post_task: PostTask) -> bool {
    unsafe {
        let mut old: libc::sigaction = std::mem::zeroed();

        if libc::sigaction(libc::SIGWINCH, std::ptr::null(), &mut old) != 0
            || old.sa_flags & libc::SA_SIGINFO != 0
            || old.sa_sigaction == libc::SIG_DFL
            || old.sa_sigaction == libc::SIG_IGN
        {
            return false;
        }

        let previous: extern "C" fn(c_int) = std::mem::transmute(old.sa_sigaction);
        let mut fds = [0; 2];

        if libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) != 0 {
            return false;
        }

        let (read_fd, write_fd) = (fds[0], fds[1]);
        WAKE_FD.store(write_fd, Ordering::SeqCst);

        let spawned = std::thread::Builder::new()
            .name("resize-signal".into())
            .spawn(move || dispatch(read_fd, post_task, previous));

        if spawned.is_err() {
            return false;
        }

        let mut new: libc::sigaction = std::mem::zeroed();
        new.sa_sigaction = on_sigwinch as usize;
        new.sa_flags = libc::SA_RESTART;
        libc::sigemptyset(&mut new.sa_mask);

        libc::sigaction(libc::SIGWINCH, &new, std::ptr::null_mut()) == 0
    }
}

fn dispatch(read_fd: c_int, post_task: PostTask, previous: extern "C" fn(c_int)) {
    let mut buf = [0u8; 64];

    loop {
        let n = unsafe { libc::read(read_fd, buf.as_mut_ptr() as *mut libc::c_void, buf.len()) };

        if n < 0 && std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
            continue;
        }
        if n <= 0 {
            return;
        }

        std::thread::sleep(COALESCE);
        // Signals from here on wake the dispatcher again; the handler reads
        // the terminal size when it runs, so no resize is lost.
        PENDING.store(false, Ordering::SeqCst);

        unsafe { super::bridge::post_task_raw(post_task, move || previous(libc::SIGWINCH)) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    use std::sync::Mutex;

    static CALLS: AtomicUsize = AtomicUsize::new(0);
    static CALLED_ON: Mutex<Option<String>> = Mutex::new(None);

    extern "C" fn chromium_handler(sig: c_int) {
        assert_eq!(sig, libc::SIGWINCH);
        CALLS.fetch_add(1, Ordering::SeqCst);
        *CALLED_ON.lock().unwrap() = std::thread::current().name().map(String::from);
    }

    /// Stands in for Chromium's post_task: runs the task right away, on the
    /// calling thread, so the test can see where the handler ran.
    extern "C" fn run_now(f: extern "C" fn(*mut libc::c_void), data: *mut libc::c_void) {
        f(data)
    }

    #[test]
    fn sigwinch_reaches_the_previous_handler_through_post_task_once_per_burst() {
        unsafe { libc::signal(libc::SIGWINCH, chromium_handler as usize) };
        assert!(route_to_main_thread(run_now));

        // A burst from another thread, like a window being dragged.
        std::thread::spawn(|| {
            for _ in 0..20 {
                unsafe { libc::raise(libc::SIGWINCH) };
            }
        })
        .join()
        .unwrap();

        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while CALLS.load(Ordering::SeqCst) == 0 && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        std::thread::sleep(COALESCE * 4);

        // Never on the signalled thread: only through post_task.
        assert_eq!(CALLED_ON.lock().unwrap().as_deref(), Some("resize-signal"));
        assert_eq!(CALLS.load(Ordering::SeqCst), 1, "a burst is one resize");

        // A later resize is delivered again.
        unsafe { libc::kill(libc::getpid(), libc::SIGWINCH) };
        std::thread::sleep(COALESCE * 6);
        assert_eq!(CALLS.load(Ordering::SeqCst), 2);
    }
}

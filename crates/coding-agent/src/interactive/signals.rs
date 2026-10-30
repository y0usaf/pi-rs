use std::sync::atomic::{AtomicBool, Ordering};

static REQUESTED: AtomicBool = AtomicBool::new(false);
const SIGNALS: [libc::c_int; 2] = [libc::SIGTERM, libc::SIGHUP];

extern "C" fn on_signal(_: libc::c_int) {
    REQUESTED.store(true, Ordering::SeqCst);
}

pub struct Shutdown {
    previous: Vec<(libc::c_int, libc::sigaction)>,
}

impl Shutdown {
    pub fn install() -> Shutdown {
        REQUESTED.store(false, Ordering::SeqCst);
        let mut previous = Vec::new();
        for signal in SIGNALS {
            unsafe {
                let mut action: libc::sigaction = std::mem::zeroed();
                action.sa_sigaction = on_signal as *const () as libc::sighandler_t;
                libc::sigemptyset(&mut action.sa_mask);
                let mut old: libc::sigaction = std::mem::zeroed();
                if libc::sigaction(signal, &action, &mut old) == 0 {
                    previous.push((signal, old));
                }
            }
        }
        Shutdown { previous }
    }

    pub fn requested(&self) -> bool {
        REQUESTED.load(Ordering::SeqCst)
    }
}

impl Drop for Shutdown {
    fn drop(&mut self) {
        for (signal, old) in &self.previous {
            unsafe {
                libc::sigaction(*signal, old, std::ptr::null_mut());
            }
        }
    }
}

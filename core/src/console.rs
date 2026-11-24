//! Keeps the terminal for the few lines that are meant for a person.
//!
//! The protocol crate prints an entire swap report to stdout with `println!` when a swap ends
//! (`taker/api.rs`), and it is not ours to patch. That report is persisted beside the wallet
//! on the very next line of the same function and rendered in the app, so the copy that lands
//! in the terminal carries nothing the user cannot already see — it only buries the one line
//! they came back for, which is the URL.
//!
//! [`detach`] therefore points file descriptor 1 at `/dev/null` and hands back a duplicate of
//! the real one. Anything printing to stdout from then on is discarded; the host writes its
//! own lines to the [`Console`] instead. Under a supervisor that duplicate is still the
//! captured pipe, so `docker logs` and journald keep seeing what we deliberately write.

use std::io::Write;

/// Where a line meant for a person goes.
pub enum Console {
    /// Stdout as it was before the redirect.
    Saved(std::fs::File),
    /// Detaching was not possible or not applicable; ordinary stdout, report and all.
    Inherited,
}

impl Console {
    pub fn line(&mut self, text: &str) {
        match self {
            Console::Saved(file) => {
                let _ = writeln!(file, "{text}");
                let _ = file.flush();
            }
            Console::Inherited => println!("{text}"),
        }
    }

    /// Points stdout back at the terminal. For a process about to `exec` its successor, which
    /// would otherwise inherit `/dev/null` and lose the lines meant for a person.
    pub fn reattach(self) {
        #[cfg(unix)]
        if let Console::Saved(file) = self {
            use std::os::fd::AsRawFd;
            // SAFETY: both descriptors are open; `file` closes its own copy on drop.
            unsafe { libc::dup2(file.as_raw_fd(), libc::STDOUT_FILENO) };
        }
    }
}

/// Sends stdout to `/dev/null` and returns the real one. Call once, before any swap can run.
///
/// Every failure path falls back to [`Console::Inherited`]: a noisy terminal is a far better
/// outcome than a host that cannot tell the user where to point their browser.
#[cfg(unix)]
pub fn detach() -> Console {
    use std::os::fd::FromRawFd;

    unsafe {
        let saved = libc::dup(libc::STDOUT_FILENO);
        if saved < 0 {
            return Console::Inherited;
        }
        let sink = libc::open(c"/dev/null".as_ptr(), libc::O_WRONLY);
        if sink < 0 {
            libc::close(saved);
            return Console::Inherited;
        }
        let replaced = libc::dup2(sink, libc::STDOUT_FILENO);
        libc::close(sink);
        if replaced < 0 {
            libc::close(saved);
            return Console::Inherited;
        }
        Console::Saved(std::fs::File::from_raw_fd(saved))
    }
}

#[cfg(not(unix))]
pub fn detach() -> Console {
    Console::Inherited
}

//! PTY management for the winit GUI: openpty + fork + exec, a reader
//! thread that forwards child output to the GUI event loop, and
//! `TIOCSWINSZ` resize propagation (the kernel then delivers SIGWINCH
//! to the child for us).
//!
//! This replaces VTE's built-in PTY. Everything is bounded and freed
//! explicitly: the reader thread owns a fixed 64 KB buffer that lives
//! exactly as long as the thread does.

use std::os::unix::io::{AsRawFd, RawFd};
use nix::pty::openpty;
use std::os::unix::io::OwnedFd;
use nix::unistd::{dup2, execvp, fork, setsid, ForkResult};

/// Events forwarded from the PTY reader thread to the GUI loop.
pub enum PtyEvent {
    /// Raw bytes from the child (already copied out of the reader's
    /// fixed buffer).
    Output(Vec<u8>),
    /// The child exited (EOF/EIO on the master fd).
    Exited,
}

/// Spawn `argv` on a fresh PTY. Returns the master-fd handle; start
/// pumping output with [`Pty::start_reader`].
///
/// Ownership: the `PtyMaster` handle is the ONLY owner of the master
/// fd — it is closed exactly once (by [`Pty::shutdown`] or by drop).
pub struct Pty {
    master: Option<OwnedFd>,
}

impl Pty {
    /// Window size reported to the child at spawn time.
    pub fn spawn(
        argv: &[String],
        cwd: &str,
        cols: u16,
        rows: u16,
        env: &[(String, String)],
    ) -> nix::Result<Self> {
        let ws = libc::winsize {
            ws_col: cols,
            ws_row: rows,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        let pty = openpty(Some(&ws), None)?;
        let master_fd = pty.master.as_raw_fd();
        let slave_fd = pty.slave.as_raw_fd();

        match unsafe { fork() }? {
            ForkResult::Child => {
                // New session: this child becomes the session leader and
                // the pty slave becomes its controlling terminal when we
                // acquire it explicitly (TIOCSCTTY).
                let _ = setsid();
                unsafe {
                    libc::ioctl(slave_fd, libc::TIOCSCTTY as libc::c_ulong, 0);
                }
                let _ = dup2(slave_fd, 0);
                let _ = dup2(slave_fd, 1);
                let _ = dup2(slave_fd, 2);
                if slave_fd > 2 {
                    // Raw close: the child exits via exec/exit below, so
                    // the nix owned handle never drops here.
                    unsafe { libc::close(slave_fd) };
                }
                // The child must NOT hold the master open, or the parent
                // never sees EOF when the shell exits.
                if master_fd > 2 {
                    unsafe { libc::close(master_fd) };
                }
                // Start in the requested working directory (HOME).
                if std::env::set_current_dir(cwd).is_err() {
                    let _ = std::env::set_current_dir("/");
                }
                for (k, v) in env {
                    std::env::set_var(k, v);
                }
                let cargs: Vec<std::ffi::CString> = argv
                    .iter()
                    .map(|a| std::ffi::CString::new(a.as_str()).unwrap_or_default())
                    .collect();
                let _ = execvp(&cargs[0], &cargs);
                // execvp only returns on failure.
                std::process::exit(127);
            }
            ForkResult::Parent { child } => {
                CHILD_PID.store(child.as_raw(), std::sync::atomic::Ordering::SeqCst);
                // Close our slave handle exactly once (plain drop): the
                // child holds the only other reference. Without this the
                // master never reports EIO when the child exits.
                drop(pty.slave);
                Ok(Self {
                    master: Some(pty.master),
                })
            }
        }
    }

    fn master_fd_opt(&self) -> Option<RawFd> {
        self.master.as_ref().map(|m| m.as_raw_fd())
    }

    pub fn master_fd(&self) -> Option<RawFd> {
        self.master_fd_opt()
    }

    /// Start the reader thread. It reads into ONE fixed 64 KB buffer
    /// (no unbounded accumulation) and forwards copies to `sender`.
    /// Raw `libc::read` is used on purpose: the fd is closed by
    /// [`Pty::shutdown`] while the thread may still be blocked in
    /// `read`, which is the standard way to break a PTY read out of
    /// its blocking state.
    pub fn start_reader(&self, sender: impl Fn(PtyEvent) + Send + 'static) {
        let fd = self.master_fd_opt();
        std::thread::spawn(move || {
            let Some(fd) = fd else { return };
            let mut buf = [0u8; 64 * 1024];
            loop {
                let n = unsafe {
                    libc::read(
                        fd,
                        buf.as_mut_ptr() as *mut libc::c_void,
                        buf.len(),
                    )
                };
                if n > 0 {
                    sender(PtyEvent::Output(buf[..n as usize].to_vec()));
                } else if n == 0 {
                    break; // EOF
                } else if std::io::Error::last_os_error().raw_os_error() == Some(libc::EINTR) {
                    continue;
                } else {
                    break; // EIO after child exit / EBADF after shutdown
                }
            }
            sender(PtyEvent::Exited);
        });
    }

    /// Write raw input bytes to the child (raw `libc::write`, see
    /// [`Pty::start_reader`] for why).
    pub fn write(&self, bytes: &[u8]) {
        let Some(master) = self.master_fd_opt() else { return };
        let mut written = 0;
        while written < bytes.len() {
            let n = unsafe {
                libc::write(
                    master,
                    bytes[written..].as_ptr() as *const libc::c_void,
                    bytes.len() - written,
                )
            };
            if n > 0 {
                written += n as usize;
            } else if n < 0 && std::io::Error::last_os_error().raw_os_error() == Some(libc::EINTR)
            {
                continue;
            } else {
                break;
            }
        }
    }

    /// Tell the child about the new grid size (kernel sends SIGWINCH).
    pub fn resize(&self, cols: u16, rows: u16) {
        let Some(master) = self.master_fd_opt() else { return };
        let ws = libc::winsize {
            ws_col: cols,
            ws_row: rows,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        unsafe {
            libc::ioctl(master, libc::TIOCSWINSZ as libc::c_ulong, &ws);
        }
    }

    /// Explicit teardown: close the master fd. The reader thread gets
    /// EBADF and exits; the child gets SIGHUP.
    pub fn shutdown(&mut self) {
        // Dropping the PtyMaster closes the fd exactly once.
        self.master.take();
    }
}

/// PID of the most recently spawned child (best-effort, for cleanup).
pub static CHILD_PID: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(-1);

/// Kill the child on window close (SIGTERM first — shells forward it).
pub fn kill_child() {
    let pid = CHILD_PID.load(std::sync::atomic::Ordering::SeqCst);
    if pid > 0 {
        unsafe {
            libc::kill(pid, libc::SIGTERM);
        }
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    /// Collect output through the callback reader into a channel.
    fn collect(pty: &Pty, _timeout: Duration) -> mpsc::Receiver<PtyEvent> {
        let (tx, rx) = mpsc::channel();
        pty.start_reader(move |ev| {
            let _ = tx.send(ev);
        });
        rx
    }

    fn recv_all(rx: &mpsc::Receiver<PtyEvent>, timeout: Duration) -> Vec<u8> {
        let deadline = Instant::now() + timeout;
        let mut out = Vec::new();
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            match rx.recv_timeout(left) {
                Ok(PtyEvent::Output(mut b)) => out.append(&mut b),
                Ok(PtyEvent::Exited) => break,
                Err(_) => break,
            }
        }
        out
    }

    #[test]
    fn pty_spawns_child_and_reads_output() {
        let mut pty = Pty::spawn(
            &["/bin/sh".into(), "-c".into(), "printf hello-pty".into()],
            "/",
            80,
            24,
            &[],
        )
        .expect("openpty");
        let rx = collect(&pty, Duration::from_secs(5));
        let out = recv_all(&rx, Duration::from_secs(5));
        assert!(String::from_utf8_lossy(&out).contains("hello-pty"), "{out:?}");
        pty.shutdown();
    }

    #[test]
    fn pty_forwards_written_input_to_the_child() {
        // cat echoes whatever we write back.
        let mut pty = Pty::spawn(&["/bin/cat".into()], "/", 80, 24, &[]).expect("openpty");
        let rx = collect(&pty, Duration::from_secs(5));
        pty.write(b"tuxixi\n");
        let out = recv_all(&rx, Duration::from_secs(5));
        assert!(String::from_utf8_lossy(&out).contains("tuxixi"), "{out:?}");
        pty.shutdown();
    }

    #[test]
    fn pty_resize_sets_winsize_for_the_child() {
        let mut pty = Pty::spawn(
            &[
                "/bin/sh".into(),
                "-c".into(),
                "sleep 0.2; stty size; exit 0".into(),
            ],
            "/",
            100,
            30,
            &[],
        )
        .expect("openpty");
        let rx = collect(&pty, Duration::from_secs(5));
        pty.resize(77, 11);
        let out = String::from_utf8_lossy(&recv_all(&rx, Duration::from_secs(5))).to_string();
        assert!(out.contains("11 77"), "stty reported: {out:?}");
        pty.shutdown();
    }

    #[test]
    fn pty_reports_exit_when_the_child_finishes() {
        let mut pty = Pty::spawn(&["/bin/true".into()], "/", 80, 24, &[]).expect("openpty");
        let rx = collect(&pty, Duration::from_secs(5));
        let mut saw_exit = false;
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            match rx.recv_timeout(Duration::from_millis(200)) {
                Ok(PtyEvent::Exited) => {
                    saw_exit = true;
                    break;
                }
                Ok(PtyEvent::Output(_)) => {}
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
        assert!(saw_exit, "reader should signal Exited after child exit");
        pty.shutdown();
    }

    #[test]
    fn pty_injects_session_env_vars() {
        let mut pty = Pty::spawn(
            &[
                "/bin/sh".into(),
                "-c".into(),
                "printf \"$TUXIXI_SESSION|$TUXIXI_BIN_DIR\"".into(),
            ],
            "/",
            80,
            24,
            &[
                ("TUXIXI_SESSION".into(), "1".into()),
                ("TUXIXI_BIN_DIR".into(), "/opt/x".into()),
            ],
        )
        .expect("openpty");
        let rx = collect(&pty, Duration::from_secs(5));
        let out = String::from_utf8_lossy(&recv_all(&rx, Duration::from_secs(5))).to_string();
        assert!(out.contains("1|/opt/x"), "{out:?}");
        pty.shutdown();
    }

    #[test]
    fn pty_child_starts_in_the_requested_cwd() {
        let dir = std::env::temp_dir().join("tuxixi-pty-cwd-test");
        std::fs::create_dir_all(&dir).unwrap();
        let mut pty = Pty::spawn(
            &["/bin/sh".into(), "-c".into(), "pwd".into()],
            dir.to_str().unwrap(),
            80,
            24,
            &[],
        )
        .expect("openpty");
        let rx = collect(&pty, Duration::from_secs(5));
        let out = String::from_utf8_lossy(&recv_all(&rx, Duration::from_secs(5))).to_string();
        assert!(out.contains("tuxixi-pty-cwd-test"), "{out:?}");
        pty.shutdown();
        std::fs::remove_dir_all(&dir).ok();
    }
}

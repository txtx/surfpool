#[cfg(unix)]
mod shutdown_tests {
    use std::io::{BufRead, BufReader};
    use std::path::{Path, PathBuf};
    use std::process::{Child, Command, Stdio};
    use std::sync::mpsc;
    use std::thread;
    use std::time::{Duration, Instant};

    fn surfpool_bin() -> String {
        std::env::var("SURFPOOL_BIN").unwrap_or_else(|_| env!("CARGO_BIN_EXE_surfpool").to_string())
    }

    /// Poll until the child process exits or the timeout elapses.
    /// Uses `waitpid(WNOHANG)` so we correctly detect zombie (exited-but-not-reaped)
    /// children -- `kill(pid, 0)` returns success for zombies, which would cause a
    /// false "still running" result.
    /// Returns `true` if the process exited within the timeout.
    fn wait_for_exit(pid: i32, timeout: Duration) -> bool {
        let start = Instant::now();
        while start.elapsed() < timeout {
            let mut status: libc::c_int = 0;
            let ret = unsafe { libc::waitpid(pid, &mut status, libc::WNOHANG) };
            if ret > 0 {
                // Child exited (or was signaled). Reaped.
                return true;
            }
            if ret < 0 {
                // ECHILD -- not our child or already reaped
                return true;
            }
            // ret == 0 means child still running
            thread::sleep(Duration::from_millis(200));
        }
        false
    }

    /// Poll until a process that is NOT our direct child exits.
    /// Uses `kill(pid, 0)` to check for liveness since we cannot `waitpid` on
    /// processes we did not fork. Returns `true` if the process disappeared
    /// within the timeout.
    fn wait_for_process_exit(pid: i32, timeout: Duration) -> bool {
        let start = Instant::now();
        while start.elapsed() < timeout {
            let ret = unsafe { libc::kill(pid, 0) };
            if ret != 0 {
                // Process gone (ESRCH) or not permitted -- either way, not running
                return true;
            }
            thread::sleep(Duration::from_millis(200));
        }
        false
    }

    // -----------------------------------------------------------------------
    // Log-mode helper
    // -----------------------------------------------------------------------

    /// Spawn surfpool in log mode (`--no-tui --yes`), wait for startup, assert
    /// it is alive, send `signal`, and assert it exits within 10 s.
    fn assert_signal_exits_log_mode(signal: i32, signal_name: &str) {
        let mut child = Command::new(surfpool_bin())
            .args(["start", "--no-tui", "--yes"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("failed to spawn surfpool");

        let pid = child.id() as i32;

        // Wait for the process to start up
        thread::sleep(Duration::from_secs(4));

        // Verify it's still running before we signal it
        assert_eq!(
            unsafe { libc::kill(pid, 0) },
            0,
            "surfpool process died before we could test it"
        );

        // Send the signal
        unsafe {
            libc::kill(pid, signal);
        }

        // The core shutdown waits up to 5s for the SVM lock + 6s for WS to finish, so allow 10s total
        // (these timeouts overlap in practice since they rarely fire).
        if !wait_for_exit(pid, Duration::from_secs(10)) {
            unsafe {
                libc::kill(pid, libc::SIGKILL);
            }
            let _ = child.wait();
            panic!("surfpool did not exit within 10s of {signal_name}");
        }
        // wait_for_exit already reaped via waitpid; process exited -- success.
    }

    // -----------------------------------------------------------------------
    // PTY / TUI-mode helpers
    // -----------------------------------------------------------------------

    /// Compile `tests/pty_launcher.c` into a temporary binary.
    /// Returns `None` (and prints a skip message) if `cc` is unavailable.
    fn compile_pty_launcher() -> Option<PathBuf> {
        let manifest_dir = env!("CARGO_MANIFEST_DIR");
        let launcher_src = Path::new(manifest_dir).join("tests/pty_launcher.c");
        let launcher_bin = std::env::temp_dir().join("pty_launcher_test");

        let mut cc_args = vec![
            "-o",
            launcher_bin.to_str().unwrap(),
            launcher_src.to_str().unwrap(),
        ];
        if cfg!(target_os = "linux") {
            cc_args.push("-lutil");
        }

        let cc_status = Command::new("cc").args(&cc_args).status();
        match cc_status {
            Ok(s) if s.success() => Some(launcher_bin),
            _ => {
                eprintln!("skipping PTY test: cc not available or compilation failed");
                None
            }
        }
    }

    /// Spawn surfpool inside `pty_launcher` (TUI mode with `--yes`).
    /// Returns the launcher `Child` and the surfpool child PID parsed from
    /// the launcher's stderr line:
    ///   `pty_launcher: master_pid=<M> child_pid=<C>`
    fn spawn_in_pty(launcher_bin: &Path) -> (Child, i32) {
        let surfpool = surfpool_bin();

        let mut launcher = Command::new(launcher_bin)
            .args([&surfpool, "start", "--yes"])
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .expect("failed to spawn pty_launcher");

        // Read the first line from stderr on a background thread so we don't
        // block forever if the launcher dies before writing it.
        let stderr = launcher.stderr.take().expect("piped stderr");
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let reader = BufReader::new(stderr);
            for line in reader.lines() {
                match line {
                    Ok(l) => {
                        let _ = tx.send(l);
                        // Keep draining so the pipe doesn't block the child,
                        // but we only care about the first message.
                    }
                    Err(_) => break,
                }
            }
        });

        // Wait up to 5 s for the banner line.
        let mut child_pid: Option<i32> = None;
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            match rx.recv_timeout(Duration::from_millis(200)) {
                Ok(line) => {
                    if let Some(pos) = line.find("child_pid=") {
                        let rest = &line[pos + "child_pid=".len()..];
                        let num: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
                        if let Ok(pid) = num.parse::<i32>() {
                            child_pid = Some(pid);
                            break;
                        }
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }

        let child_pid = child_pid.expect("failed to parse child_pid from pty_launcher stderr");

        (launcher, child_pid)
    }

    // -----------------------------------------------------------------------
    // Log-mode tests
    // -----------------------------------------------------------------------

    #[test]
    fn sigint_causes_clean_exit() {
        assert_signal_exits_log_mode(libc::SIGINT, "SIGINT");
    }

    #[test]
    fn sigterm_causes_clean_exit() {
        assert_signal_exits_log_mode(libc::SIGTERM, "SIGTERM");
    }

    #[test]
    fn sighup_causes_clean_exit() {
        assert_signal_exits_log_mode(libc::SIGHUP, "SIGHUP");
    }

    #[test]
    fn sigterm_with_db_causes_clean_exit() {
        let temp_dir = std::env::temp_dir();
        let db_path = temp_dir.join(format!("surfpool_test_{}.sqlite", std::process::id()));

        let mut child = Command::new(surfpool_bin())
            .args([
                "start",
                "--no-tui",
                "--yes",
                "--db",
                db_path.to_str().unwrap(),
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("failed to spawn surfpool");

        let pid = child.id() as i32;
        thread::sleep(Duration::from_secs(4));

        // Check if process died during startup (e.g., due to tokio runtime panic)
        let startup_check = unsafe { libc::kill(pid, 0) };
        if startup_check != 0 {
            // Process died - capture output for debugging
            let output = child.wait_with_output().expect("failed to get output");
            let stderr = String::from_utf8_lossy(&output.stderr);
            let _ = std::fs::remove_file(&db_path);
            panic!(
                "surfpool process died during startup.\nstderr: {}",
                stderr
            );
        }

        unsafe {
            libc::kill(pid, libc::SIGTERM);
        }

        if !wait_for_exit(pid, Duration::from_secs(10)) {
            unsafe {
                libc::kill(pid, libc::SIGKILL);
            }
            let _ = child.wait();
            let _ = std::fs::remove_file(&db_path);
            panic!("surfpool with --db did not exit within 10s of SIGTERM");
        }

        // Cleanup db file
        let _ = std::fs::remove_file(&db_path);
    }

    // -----------------------------------------------------------------------
    // TUI-mode tests (PTY)
    // -----------------------------------------------------------------------

    #[test]
    fn pty_close_causes_clean_exit() {
        let launcher_bin = match compile_pty_launcher() {
            Some(p) => p,
            None => return,
        };
        let (mut launcher, child_pid) = spawn_in_pty(&launcher_bin);

        // Wait for surfpool to start up inside the PTY
        thread::sleep(Duration::from_secs(4));

        // Kill the launcher -- this closes the PTY master fd, which should
        // cause SIGHUP + EIO on the slave side, triggering surfpool to exit.
        let launcher_pid = launcher.id() as i32;
        unsafe {
            libc::kill(launcher_pid, libc::SIGTERM);
        }
        let _ = launcher.wait();

        // The surfpool child is an orphan (reparented to init), so use
        // kill(pid, 0) polling instead of waitpid.
        if !wait_for_process_exit(child_pid, Duration::from_secs(10)) {
            // Clean up
            unsafe {
                libc::kill(child_pid, libc::SIGKILL);
            }
            panic!("surfpool did not exit within 10s of PTY close");
        }
    }

    #[test]
    fn tui_sigint_causes_clean_exit() {
        let launcher_bin = match compile_pty_launcher() {
            Some(p) => p,
            None => return,
        };
        let (mut launcher, child_pid) = spawn_in_pty(&launcher_bin);

        // Wait for surfpool to start up inside the PTY
        thread::sleep(Duration::from_secs(4));

        // Send SIGINT directly to the surfpool child running in the PTY
        unsafe {
            libc::kill(child_pid, libc::SIGINT);
        }

        if !wait_for_process_exit(child_pid, Duration::from_secs(10)) {
            unsafe {
                libc::kill(child_pid, libc::SIGKILL);
            }
            // Also clean up the launcher
            unsafe {
                libc::kill(launcher.id() as i32, libc::SIGKILL);
            }
            let _ = launcher.wait();
            panic!("surfpool did not exit within 10s of SIGINT in TUI mode");
        }

        // Clean up the launcher
        unsafe {
            libc::kill(launcher.id() as i32, libc::SIGTERM);
        }
        let _ = launcher.wait();
    }

    #[test]
    fn tui_sigterm_causes_clean_exit() {
        let launcher_bin = match compile_pty_launcher() {
            Some(p) => p,
            None => return,
        };
        let (mut launcher, child_pid) = spawn_in_pty(&launcher_bin);

        // Wait for surfpool to start up inside the PTY
        thread::sleep(Duration::from_secs(4));

        // Send SIGTERM directly to the surfpool child running in the PTY
        unsafe {
            libc::kill(child_pid, libc::SIGTERM);
        }

        if !wait_for_process_exit(child_pid, Duration::from_secs(10)) {
            unsafe {
                libc::kill(child_pid, libc::SIGKILL);
            }
            // Also clean up the launcher
            unsafe {
                libc::kill(launcher.id() as i32, libc::SIGKILL);
            }
            let _ = launcher.wait();
            panic!("surfpool did not exit within 10s of SIGTERM in TUI mode");
        }

        // Clean up the launcher
        unsafe {
            libc::kill(launcher.id() as i32, libc::SIGTERM);
        }
        let _ = launcher.wait();
    }
}

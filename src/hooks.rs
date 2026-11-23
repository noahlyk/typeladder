//! Hooks: shell commands run around a session or a test, so a text expander or an engine can be
//! paused while you practise (`espanso stop` before, `espanso start` after).
//!
//! `before_session` and `after_session` run once around the whole run; `after_session` also runs
//! when the run is quit, interrupted (Ctrl-C, SIGTERM), fails or panics, once, best effort.
//! `before_test` and `after_test` run around each test. Each command goes through `sh -c` with no
//! input and no output, and is given [`TIMEOUT`]; a failure or a timeout is a one-line warning,
//! never an error.

use crate::config;
use std::cell::RefCell;
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// How long a hook may run before it is killed.
pub const TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    BeforeSession,
    AfterSession,
    BeforeTest,
    AfterTest,
}

impl Event {
    const fn name(self) -> &'static str {
        match self {
            Self::BeforeSession => "before_session",
            Self::AfterSession => "after_session",
            Self::BeforeTest => "before_test",
            Self::AfterTest => "after_test",
        }
    }
}

/// Runs `command` through `sh -c`; the error is the one-line warning.
pub fn run_command(name: &str, command: &str, timeout: Duration) -> Result<(), String> {
    let mut child = Command::new("sh")
        .arg("-c")
        .arg(command)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("hook {name} could not start: {e}"))?;
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return Ok(()),
            Ok(Some(status)) => return Err(format!("hook {name} failed: {command:?} {status}")),
            Ok(None) if start.elapsed() >= timeout => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!(
                    "hook {name} timed out after {}s and was stopped: {command:?}",
                    timeout.as_secs()
                ));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(e) => return Err(format!("hook {name} could not be waited for: {e}")),
        }
    }
}

/// The configured hooks. Warnings from hooks that run while the screen is up are kept to be
/// printed after it closes ([`Hooks::take_warnings`]).
pub struct Hooks {
    cfg: config::Hooks,
    enabled: bool,
    warnings: RefCell<Vec<String>>,
}

impl Hooks {
    #[must_use]
    pub const fn new(cfg: config::Hooks, enabled: bool) -> Self {
        Self {
            cfg,
            enabled,
            warnings: RefCell::new(Vec::new()),
        }
    }

    /// No hooks at all.
    #[cfg(test)]
    #[must_use]
    pub fn off() -> Self {
        Self::new(config::Hooks::default(), false)
    }

    fn command(&self, event: Event) -> Option<&str> {
        let c = match event {
            Event::BeforeSession => &self.cfg.before_session,
            Event::AfterSession => &self.cfg.after_session,
            Event::BeforeTest => &self.cfg.before_test,
            Event::AfterTest => &self.cfg.after_test,
        };
        c.as_deref().map(str::trim).filter(|c| !c.is_empty() && self.enabled)
    }

    /// Runs the hook of `event`, if there is one; a failure is kept as a warning.
    pub fn run(&self, event: Event) {
        if let Some(command) = self.command(event)
            && let Err(w) = run_command(event.name(), command, TIMEOUT)
        {
            self.warnings.borrow_mut().push(w);
        }
    }

    pub fn take_warnings(&self) -> Vec<String> {
        std::mem::take(&mut *self.warnings.borrow_mut())
    }

    /// Starts a session: runs `before_session` (a failure is printed at once) and arranges for
    /// `after_session` to run exactly once when the returned guard is dropped, or at Ctrl-C.
    #[must_use]
    pub fn start_session(&self) -> SessionGuard {
        self.run(Event::BeforeSession);
        for w in self.take_warnings() {
            eprintln!("{w}");
        }
        if let Some(after) = self.command(Event::AfterSession) {
            *AFTER.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(after.to_owned());
            // best effort: a second handler in the same process (tests) is not an error
            let _ = ctrlc::set_handler(|| {
                ratatui::restore();
                finish_session();
                std::process::exit(130);
            });
        }
        SessionGuard
    }
}

/// The `after_session` command still to run; taken by whoever gets there first.
static AFTER: Mutex<Option<String>> = Mutex::new(None);

/// Runs `after_session` if it has not run yet.
pub fn finish_session() {
    let command = AFTER.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take();
    if let Some(command) = command
        && let Err(w) = run_command("after_session", &command, TIMEOUT)
    {
        eprintln!("{w}");
    }
}

/// Runs `after_session` when dropped: on a normal end, an error and a panic alike.
pub struct SessionGuard;

impl Drop for SessionGuard {
    fn drop(&mut self) {
        finish_session();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> std::path::PathBuf {
        let base = std::env::var_os("TMPDIR").map_or_else(std::env::temp_dir, std::path::PathBuf::from);
        let dir = base.join(format!("typeladder-hooks-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn hooks(before_test: &str) -> Hooks {
        Hooks::new(
            config::Hooks {
                before_test: Some(before_test.to_owned()),
                ..config::Hooks::default()
            },
            true,
        )
    }

    #[test]
    fn a_hook_runs_its_command_through_the_shell() {
        let dir = scratch("run");
        let file = dir.join("out.txt");
        let h = hooks(&format!("echo hi > {}", file.display()));
        h.run(Event::BeforeTest);
        assert_eq!(std::fs::read_to_string(&file).unwrap().trim(), "hi");
        assert_eq!(h.take_warnings().len(), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_failing_hook_is_a_one_line_warning_not_an_error() {
        let h = hooks("exit 3");
        h.run(Event::BeforeTest);
        let w = h.take_warnings();
        assert_eq!(w.len(), 1);
        assert!(w[0].contains("before_test failed") && !w[0].contains('\n'), "{w:?}");
        assert!(h.take_warnings().is_empty(), "taken once");
    }

    #[test]
    fn a_hook_that_hangs_is_killed_at_the_timeout() {
        let start = Instant::now();
        let err = run_command("before_test", "sleep 30", Duration::from_millis(200)).unwrap_err();
        assert!(err.contains("timed out"), "{err}");
        assert!(start.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn hooks_off_or_unset_or_blank_run_nothing() {
        let dir = scratch("off");
        let file = dir.join("never.txt");
        let cmd = format!("touch {}", file.display());
        Hooks::new(
            config::Hooks {
                before_test: Some(cmd.clone()),
                ..config::Hooks::default()
            },
            false,
        )
        .run(Event::BeforeTest);
        hooks(&cmd).run(Event::AfterTest);
        hooks("   ").run(Event::BeforeTest);
        Hooks::off().run(Event::BeforeTest);
        assert!(!file.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_session_guard_runs_after_session_once_even_on_a_panic() {
        let dir = scratch("guard");
        let file = dir.join("count.txt");
        let h = Hooks::new(
            config::Hooks {
                before_session: Some(format!("echo b >> {}", file.display())),
                after_session: Some(format!("echo a >> {}", file.display())),
                ..config::Hooks::default()
            },
            true,
        );
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let guard = h.start_session();
            let _keep = &guard;
            panic!("boom");
        }));
        assert!(result.is_err());
        finish_session();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "b\na\n", "before once, after once");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

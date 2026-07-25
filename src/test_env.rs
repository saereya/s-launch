//! Shared helper for tests that mutate process-global environment variables
//! (HOME, PATH, XDG_*). Env vars are process state, and `cargo test` runs
//! tests in parallel threads within one process, so any test touching these
//! must serialize against every other such test project-wide, not just
//! within its own file — hence one lock shared via `pub(crate)` across all
//! `#[cfg(test)]` modules.
use std::sync::{Mutex, MutexGuard, OnceLock};

static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

/// Hold this guard for the duration of any test that reads/writes env vars
/// consulted by production code (HOME, PATH, XDG_DATA_DIRS, XDG_RUNTIME_DIR,
/// XDG_CONFIG_HOME, ...).
pub fn lock() -> MutexGuard<'static, ()> {
    ENV_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Sets an env var and restores its previous value (or removes it) on drop.
pub struct EnvVarGuard {
    key: String,
    previous: Option<String>,
}

impl EnvVarGuard {
    /// # Safety
    /// Caller must hold `lock()` for the lifetime of this guard: setting env
    /// vars is only sound when no other thread can observe or race the change.
    pub unsafe fn set(key: &str, value: &str) -> Self {
        let previous = std::env::var(key).ok();
        unsafe { std::env::set_var(key, value) };
        Self {
            key: key.to_string(),
            previous,
        }
    }

    /// # Safety
    /// Caller must hold `lock()` for the lifetime of this guard.
    pub unsafe fn remove(key: &str) -> Self {
        let previous = std::env::var(key).ok();
        unsafe { std::env::remove_var(key) };
        Self {
            key: key.to_string(),
            previous,
        }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        match &self.previous {
            Some(v) => unsafe { std::env::set_var(&self.key, v) },
            None => unsafe { std::env::remove_var(&self.key) },
        }
    }
}

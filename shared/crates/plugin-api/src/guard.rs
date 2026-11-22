//! Panic containment around a machine's calls.
//!
//! A panic inside a machine must not take the whole program (and its debugger,
//! save-state writer and disk exit policies) down with no explanation. Every
//! call a plugin answers is wrapped in [`guarded`] / [`guarded_call`]: a panic
//! is caught, the first one is remembered (its text is what the host shows),
//! and the call returns an error value. After a crash the machine is treated
//! as dead -- see [`has_crashed`] -- and its fast-path functions report a
//! halted machine rather than looping forever.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Mutex;

static CRASH: Mutex<Option<String>> = Mutex::new(None);

/// The text of the first panic caught, if any.
#[must_use]
pub fn crash_message() -> Option<String> {
    CRASH
        .lock()
        .map_or_else(|e| e.into_inner().clone(), |g| g.clone())
}

/// Whether a panic was ever caught.
#[must_use]
pub fn has_crashed() -> bool {
    crash_message().is_some()
}

fn payload_text(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(text) = payload.downcast_ref::<&str>() {
        (*text).to_string()
    } else if let Some(text) = payload.downcast_ref::<String>() {
        text.clone()
    } else {
        "unknown panic".to_string()
    }
}

fn record(site: &str, payload: &(dyn std::any::Any + Send)) -> String {
    let text = format!("panic in {site}: {}", payload_text(payload));
    let mut slot = CRASH.lock().unwrap_or_else(|e| e.into_inner());
    if slot.is_none() {
        *slot = Some(text.clone());
    }
    text
}

/// Runs `f`; a panic is recorded and `on_panic` returned instead.
pub fn guarded<T>(site: &str, on_panic: T, f: impl FnOnce() -> T) -> T {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(value) => value,
        Err(payload) => {
            record(site, payload.as_ref());
            on_panic
        }
    }
}

/// Runs a call that returns a `Result`; a panic (or an earlier crash) becomes
/// an error carrying the text.
pub fn guarded_call<T>(site: &str, f: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    if let Some(message) = crash_message() {
        return Err(format!("the plugin crashed earlier ({message})"));
    }
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(result) => result,
        Err(payload) => Err(record(site, payload.as_ref())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // One test only: the crash record is process-global by design.
    #[test]
    fn a_panic_is_contained_remembered_and_turns_later_calls_into_errors() {
        assert!(!has_crashed());
        assert_eq!(guarded("fine", 1, || 7), 7);
        assert!(!has_crashed());

        let value = guarded("step", 4u32, || -> u32 { panic!("index out of range") });
        assert_eq!(value, 4);
        assert_eq!(
            crash_message().as_deref(),
            Some("panic in step: index out of range")
        );

        // A second panic does not replace the first message.
        let _ = guarded("pc", 0u16, || -> u16 { panic!("later") });
        assert_eq!(
            crash_message().as_deref(),
            Some("panic in step: index out of range")
        );

        // Calls after a crash are refused with the original text.
        let error = guarded_call("instance_call", || Ok(())).unwrap_err();
        assert!(error.contains("crashed earlier"), "{error}");
        assert!(error.contains("index out of range"), "{error}");
    }
}

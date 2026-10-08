pub mod cli;
pub mod inputs;
pub mod outputs;
pub mod protocol;
pub mod supervisor;
pub mod tui;
pub mod webui;
use crate::support::{CLEANUP_FAILED, INTERRUPTED};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::Ordering;
pub struct Suite {
    pub passed: usize,
    pub failed: usize,
    pub skipped: usize,
    pub filter: Option<String>,
}
impl Suite {
    pub fn new(filter: Option<String>) -> Self {
        Self {
            passed: 0,
            failed: 0,
            skipped: 0,
            filter,
        }
    }
    pub fn case(&mut self, name: &str, f: impl FnOnce()) {
        if self.filter.as_ref().is_some_and(|v| !name.contains(v)) {
            return;
        }
        CLEANUP_FAILED.store(false, Ordering::Relaxed);
        println!("RUN {name}");
        let passed = catch_unwind(AssertUnwindSafe(f)).is_ok();
        if passed && !CLEANUP_FAILED.swap(false, Ordering::Relaxed) {
            self.passed += 1;
            println!("PASS {name}")
        } else {
            self.failed += 1;
            println!("FAIL {name}")
        }
        assert!(
            !INTERRUPTED.load(std::sync::atomic::Ordering::Relaxed),
            "interrupted"
        );
    }
    pub fn skip(&mut self, name: &str, reason: &str) {
        self.skipped += 1;
        println!("SKIP {name}: {reason}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleanup_failure_cannot_pass_and_does_not_poison_next_case() {
        let mut suite = Suite::new(None);
        suite.case("failed cleanup", || {
            CLEANUP_FAILED.store(true, Ordering::Relaxed)
        });
        suite.case("clean", || {});
        assert_eq!((suite.passed, suite.failed), (1, 1));
    }
}

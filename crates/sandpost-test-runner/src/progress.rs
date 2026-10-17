use std::{
    io::{self, IsTerminal, Write},
    time::Instant,
};

/// Report sequential smoke phases, retaining the active phase until it succeeds or fails.
pub struct Progress<Output: Write = io::Stderr> {
    output: Output,
    color: bool,
    started: Instant,
    phase_count: usize,
    active: Option<(String, Instant)>,
}

impl Progress {
    /// Use color only on an interactive terminal that has not disabled it.
    pub fn new() -> Self {
        let output = io::stderr();
        let color = output.is_terminal()
            && std::env::var_os("NO_COLOR").is_none()
            && std::env::var_os("TERM").is_none_or(|value| value != "dumb");
        Self::with_output(output, color)
    }
}

impl<Output: Write> Progress<Output> {
    /// Select a writer so progress and failure reporting can be checked without a terminal.
    fn with_output(output: Output, color: bool) -> Self {
        Self {
            output,
            color,
            started: Instant::now(),
            phase_count: 0,
            active: None,
        }
    }

    /// Mark the previous phase successful and immediately display the next running phase.
    pub fn start(&mut self, name: &str) {
        self.complete_active("PASS", "32");
        self.phase_count += 1;
        self.write_status("RUN ", "33", name, None);
        self.active = Some((name.to_owned(), Instant::now()));
    }

    /// Mark the final phase successful and display the overall elapsed time.
    pub fn finish(&mut self) {
        self.complete_active("PASS", "32");
        let _ = writeln!(
            self.output,
            "Completed {} phases in {:.3}s",
            self.phase_count,
            self.started.elapsed().as_secs_f64()
        );
        let _ = self.output.flush();
    }

    /// Consume an active phase exactly once, reporting its elapsed time and final status.
    fn complete_active(&mut self, status: &str, color: &str) {
        if let Some((name, started)) = self.active.take() {
            self.write_status(status, color, &name, Some(started.elapsed().as_secs_f64()));
        }
    }

    /// Write append-only progress lines that remain readable in redirected CI logs.
    fn write_status(&mut self, status: &str, color: &str, name: &str, seconds: Option<f64>) {
        if self.color {
            let _ = write!(self.output, "\x1b[{color}m[{status}]\x1b[0m");
        } else {
            let _ = write!(self.output, "[{status}]");
        }
        let _ = write!(self.output, " {:02} {name}", self.phase_count);
        if let Some(seconds) = seconds {
            let _ = write!(self.output, " ({seconds:.3}s)");
        }
        let _ = writeln!(self.output);
        // A closed output pipe must not interrupt server cleanup or replace a test failure.
        let _ = self.output.flush();
    }
}

impl<Output: Write> Drop for Progress<Output> {
    /// Identify the interrupted phase on both returned errors and assertion unwinding.
    fn drop(&mut self) {
        self.complete_active("FAIL", "31");
    }
}

#[cfg(test)]
mod tests {
    use super::Progress;

    /// Report each successful phase once, with timings and a total, without terminal escapes.
    #[test]
    fn completed_phases_report_success_and_elapsed_time() {
        let mut output = Vec::new();
        {
            let mut progress = Progress::with_output(&mut output, false);
            progress.start("Startup");
            progress.start("SMTP");
            progress.finish();
        }
        let output = String::from_utf8(output).unwrap();
        assert!(output.starts_with("[RUN ] 01 Startup\n[PASS] 01 Startup ("));
        assert!(output.contains("[RUN ] 02 SMTP\n[PASS] 02 SMTP ("));
        assert!(output.contains("Completed 2 phases in "));
        assert_eq!(output.matches("[PASS]").count(), 2);
        assert!(!output.contains("FAIL"));
        assert!(!output.contains('\u{1b}'));
    }

    /// Preserve the failing phase when a check returns an error before the next phase starts.
    #[test]
    fn returned_errors_report_the_active_phase() {
        let mut output = Vec::new();
        let result: Result<(), &str> = (|| {
            let mut progress = Progress::with_output(&mut output, false);
            progress.start("Startup");
            progress.start("Indexing");
            Err("index did not converge")?;
            Ok(())
        })();
        assert!(result.is_err());
        let output = String::from_utf8(output).unwrap();
        assert_eq!(output.matches("[PASS]").count(), 1);
        assert!(output.contains("[FAIL] 02 Indexing ("));
        assert!(!output.contains("Completed"));
    }

    /// Preserve failure reporting during assertion unwinding without masking the original panic.
    #[test]
    fn assertion_failures_report_the_active_phase() {
        let mut output = Vec::new();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut progress = Progress::with_output(&mut output, false);
            progress.start("IMAP persistence");
            panic!("UIDVALIDITY changed");
        }));
        assert!(result.is_err());
        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("[FAIL] 01 IMAP persistence ("));
        assert!(!output.contains("[PASS]"));
        assert!(!output.contains("Completed"));
    }
}

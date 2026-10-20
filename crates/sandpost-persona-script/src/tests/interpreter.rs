use super::compiled;
use crate::{ActionHandler, Condition, LinkSelector, Step, execute};
use async_trait::async_trait;
use sandpost_core::{Mailbox, MessageFacts};
use std::time::Duration;

/// Records every action and can stop the run at a chosen step.
#[derive(Default)]
struct Recorder {
    events: Vec<String>,
    halt_at: Option<u32>,
    sleep: bool,
}

impl Recorder {
    /**
     * Record one event, stopping the run when the step is the configured halt step.
     */
    fn record(&mut self, step: Step, event: String) -> Result<(), String> {
        self.events.push(format!("{}:{event}", step.index));
        if self.halt_at == Some(step.index) {
            Err(format!("halted at {}", step.index))
        } else {
            Ok(())
        }
    }
}

#[async_trait]
impl ActionHandler for Recorder {
    type Halt = String;

    /**
     * Record a condition outcome.
     */
    async fn branch(
        &mut self,
        step: Step,
        condition: &Condition,
        outcome: bool,
    ) -> Result<(), String> {
        self.record(step, format!("if {condition} => {outcome}"))
    }

    /**
     * Record an open.
     */
    async fn open(&mut self, step: Step) -> Result<(), String> {
        self.record(step, "open".into())
    }

    /**
     * Record a wait, optionally sleeping on the Tokio timer.
     */
    async fn wait(&mut self, step: Step, duration: Duration) -> Result<(), String> {
        if self.sleep {
            tokio::time::sleep(duration).await;
        }
        self.record(step, format!("wait {}ms", duration.as_millis()))
    }

    /**
     * Record a click.
     */
    async fn click(&mut self, step: Step, selector: &LinkSelector) -> Result<(), String> {
        self.record(step, format!("click {selector}"))
    }
}

/**
 * Build facts for a message with the given subject and sender domain.
 */
fn facts(subject: &str, sender_domain: &str) -> MessageFacts {
    MessageFacts {
        subject: subject.into(),
        from: vec![Mailbox {
            address: format!("sender@{sender_domain}"),
            domain: sender_domain.into(),
        }],
        ..MessageFacts::default()
    }
}

#[tokio::test]
/**
 * Verify statements run strictly in order and conditions choose the right branch.
 */
async fn sequential_execution_and_branches() {
    let program = compiled(
        r#"wait 5s; open; if (subject contains "Verify") { wait 2s; click link("a.verify"); } else if (from.domain == "example.com") { click link(text: "Hi"); } else { wait 30s; }"#,
    );
    let mut recorder = Recorder::default();
    let report = execute(&program, &facts("Please Verify", "x.test"), &mut recorder)
        .await
        .unwrap();
    assert_eq!(
        recorder.events,
        [
            "0:wait 5000ms",
            "1:open",
            "2:if subject contains \"Verify\" => true",
            "3:wait 2000ms",
            "4:click link(\"a.verify\")"
        ]
    );
    assert_eq!(report.steps, 5);

    let mut recorder = Recorder::default();
    execute(&program, &facts("Hello", "EXAMPLE.com"), &mut recorder)
        .await
        .unwrap();
    assert_eq!(
        recorder.events[2..],
        [
            "2:if subject contains \"Verify\" => false",
            "3:if from.domain == \"example.com\" => true",
            "4:click link(text: \"Hi\")"
        ]
    );

    let mut recorder = Recorder::default();
    execute(&program, &facts("Hello", "other.test"), &mut recorder)
        .await
        .unwrap();
    assert_eq!(recorder.events.last().unwrap(), "4:wait 30000ms");
}

#[tokio::test]
/**
 * Verify a halted action stops the run and later statements never start.
 */
async fn halting_stops_later_actions() {
    let program = compiled("open; click link(\"a\"); wait 1s; open;");
    let mut recorder = Recorder {
        halt_at: Some(1),
        ..Recorder::default()
    };
    let error = execute(&program, &MessageFacts::default(), &mut recorder)
        .await
        .unwrap_err();
    assert_eq!(error, "halted at 1");
    assert_eq!(recorder.events, ["0:open", "1:click link(\"a\")"]);
}

#[tokio::test]
/**
 * Verify identical inputs replay identical step numbers, which restart recovery relies on.
 */
async fn step_numbers_are_deterministic() {
    let program =
        compiled("if (true) { open; } wait 1ms; if (false) { open; } else { click link(\"a\"); }");
    let mut first = Recorder::default();
    let mut second = Recorder::default();
    execute(&program, &MessageFacts::default(), &mut first)
        .await
        .unwrap();
    execute(&program, &MessageFacts::default(), &mut second)
        .await
        .unwrap();
    assert_eq!(first.events, second.events);
    assert_eq!(first.events.len(), 5);
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
/**
 * Verify many long waits run concurrently on one thread without blocking it.
 */
async fn waits_do_not_block_threads() {
    let program = std::sync::Arc::new(compiled("wait 10m; open;"));
    let started = tokio::time::Instant::now();
    let mut tasks = tokio::task::JoinSet::new();
    for _ in 0..200 {
        let program = program.clone();
        tasks.spawn(async move {
            let mut recorder = Recorder {
                sleep: true,
                ..Recorder::default()
            };
            execute(&program, &MessageFacts::default(), &mut recorder)
                .await
                .unwrap();
            recorder.events
        });
    }
    while let Some(events) = tasks.join_next().await {
        assert_eq!(events.unwrap(), ["0:wait 600000ms", "1:open"]);
    }
    assert_eq!(started.elapsed(), Duration::from_secs(600));
}

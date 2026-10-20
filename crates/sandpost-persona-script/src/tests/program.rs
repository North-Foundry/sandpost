use super::compiled;
use crate::{Operation, Program};
use std::time::Duration;

#[test]
/**
 * Verify `else if` chains lower to forward branches and jumps with correct targets.
 */
fn conditionals_lower_to_forward_jumps() {
    let program = compiled(
        "open; if (true) { wait 1s; } else if (false) { wait 2s; } else { wait 3s; } click link(\"a\");",
    );
    let operations: Vec<_> = program
        .instructions()
        .iter()
        .map(|instruction| match &instruction.operation {
            Operation::Open => "open".to_owned(),
            Operation::Wait(duration) => format!("wait {}", duration.as_secs()),
            Operation::Click(_) => "click".to_owned(),
            Operation::Branch { otherwise, .. } => format!("branch {otherwise}"),
            Operation::Jump(target) => format!("jump {target}"),
        })
        .collect();
    assert_eq!(
        operations,
        [
            "open", "branch 4", "wait 1", "jump 8", "branch 7", "wait 2", "jump 8", "wait 3",
            "click"
        ]
    );
    assert_eq!(program.statement_count(), 7);
    assert_eq!(program.longest_total_wait(), Duration::from_secs(3));
}

#[test]
/**
 * Verify the longest total wait takes the maximum over branches and sums sequences.
 */
fn longest_total_wait_follows_the_slowest_path() {
    let program = compiled(
        "wait 1s; if (true) { wait 10s; if (true) { wait 5s; } } else { wait 12s; } wait 2s;",
    );
    assert_eq!(program.longest_total_wait(), Duration::from_secs(18));
}

#[test]
/**
 * Verify compiled programs can be shared between asynchronous tasks.
 */
fn programs_are_send_and_sync() {
    /**
     * Require a type that may cross threads.
     */
    fn require<Value: Send + Sync + 'static>() {}
    require::<Program>();
}

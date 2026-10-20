//! Sequential execution of a compiled program through caller-supplied actions.
use crate::diagnostics::Span;
use crate::program::{Operation, Program};
use crate::syntax::{Condition, LinkSelector};
use async_trait::async_trait;
use sandpost_core::MessageFacts;
use std::time::Duration;

/// The position of one executed action within a run.
///
/// Steps are numbered from zero in execution order, counting every executed `open`, `wait`,
/// `click`, and evaluated condition. Re-running the same program against the same facts yields
/// the same steps, which lets a handler recognize actions it already performed before a restart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Step {
    pub index: u32,
    pub span: Span,
}

/// Performs the effects of a program. Each method returns only after its action has finished, so
/// the next action never starts early.
///
/// Returning `Err` stops the run immediately and is passed back from [`execute`]; handlers use it
/// for failed actions, cancellation, revoked authorization, and timeouts.
#[async_trait]
pub trait ActionHandler: Send {
    /// Why a handler stopped the run.
    type Halt: Send;

    /**
     * Observe the outcome of a condition before its branch runs.
     */
    async fn branch(
        &mut self,
        step: Step,
        condition: &Condition,
        outcome: bool,
    ) -> Result<(), Self::Halt>;

    /**
     * Open the message.
     */
    async fn open(&mut self, step: Step) -> Result<(), Self::Halt>;

    /**
     * Wait for `duration` without blocking a thread.
     */
    async fn wait(&mut self, step: Step, duration: Duration) -> Result<(), Self::Halt>;

    /**
     * Select a link of the message and follow it.
     */
    async fn click(&mut self, step: Step, selector: &LinkSelector) -> Result<(), Self::Halt>;
}

/// What a completed run did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExecutionReport {
    /// Steps executed, including evaluated conditions.
    pub steps: u32,
}

/**
 * Run a program against one message's facts, one instruction after another.
 *
 * Conditions are evaluated with `sandpost-query` semantics against `facts`; actions go to
 * `handler` in program order and each is awaited before the next begins.
 */
pub async fn execute<Handler>(
    program: &Program,
    facts: &MessageFacts,
    handler: &mut Handler,
) -> Result<ExecutionReport, Handler::Halt>
where
    Handler: ActionHandler + ?Sized,
{
    let instructions = program.instructions();
    let mut counter = 0;
    let mut step_index = 0u32;
    while let Some(instruction) = instructions.get(counter) {
        let step = Step {
            index: step_index,
            span: instruction.span,
        };
        counter = match &instruction.operation {
            Operation::Open => {
                handler.open(step).await?;
                counter + 1
            }
            Operation::Wait(duration) => {
                handler.wait(step, *duration).await?;
                counter + 1
            }
            Operation::Click(selector) => {
                handler.click(step, selector).await?;
                counter + 1
            }
            Operation::Branch {
                condition,
                otherwise,
            } => {
                let outcome = condition.query.evaluate(facts);
                handler.branch(step, condition, outcome).await?;
                if outcome { counter + 1 } else { *otherwise }
            }
            Operation::Jump(target) => {
                debug_assert!(*target > counter, "jumps only move forward");
                *target
            }
        };
        if !matches!(instruction.operation, Operation::Jump(_)) {
            step_index += 1;
        }
    }
    Ok(ExecutionReport { steps: step_index })
}

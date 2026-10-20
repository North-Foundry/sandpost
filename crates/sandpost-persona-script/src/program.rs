//! Lowering of the syntax tree into a flat, forward-only instruction sequence.
use crate::diagnostics::{Diagnostic, ScriptError, SourceMap, Span};
use crate::parser::{describe_duration, parse};
use crate::syntax::{
    Alternative, Condition, LinkSelector, Script, ScriptLimits, Statement, StatementKind,
};
use std::time::Duration;

/// A compiled script, ready to run against any number of messages.
///
/// Programs are immutable and cheap to share; compile once when a script is saved and reuse the
/// result for every message instead of parsing the source again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Program {
    instructions: Vec<Instruction>,
    statement_count: usize,
    longest_total_wait: Duration,
}

/// One instruction and the statement it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Instruction {
    pub operation: Operation,
    pub span: Span,
}

/// An instruction's effect.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Operation {
    Open,
    Wait(Duration),
    Click(LinkSelector),
    /// Continue with the next instruction when the condition holds, otherwise jump to
    /// `otherwise`. The target is always later in the program.
    Branch {
        condition: Condition,
        otherwise: usize,
    },
    /// Continue at a later instruction, skipping an `else` branch.
    Jump(usize),
}

/**
 * Parse, validate, and compile a script.
 *
 * Besides the parser's checks, the longest possible sum of waits along any path must stay within
 * `limits.maximum_total_wait`, so an accepted script can finish within its execution budget.
 */
pub fn compile(source: &str, limits: &ScriptLimits) -> Result<Program, ScriptError> {
    let script = parse(source, limits)?;
    let longest_total_wait = longest_wait(&script.statements);
    if longest_total_wait > limits.maximum_total_wait {
        let map = SourceMap::new(source);
        return Err(map.error(Diagnostic::at(
            0,
            format!(
                "the script can wait {} in total, more than the maximum of {}",
                describe_duration(longest_total_wait),
                describe_duration(limits.maximum_total_wait)
            ),
        )));
    }
    Ok(Program::from_script(&script))
}

impl Program {
    /**
     * Lower a validated syntax tree into instructions with forward jumps.
     */
    pub fn from_script(script: &Script) -> Self {
        let mut instructions = Vec::new();
        let mut statement_count = 0;
        lower(&script.statements, &mut instructions, &mut statement_count);
        Self {
            instructions,
            statement_count,
            longest_total_wait: longest_wait(&script.statements),
        }
    }

    /**
     * Return the instructions in execution order.
     */
    pub fn instructions(&self) -> &[Instruction] {
        &self.instructions
    }

    /**
     * Return how many statements the script contains, including nested ones.
     */
    pub fn statement_count(&self) -> usize {
        self.statement_count
    }

    /**
     * Return the longest sum of waits along any path through the script.
     */
    pub fn longest_total_wait(&self) -> Duration {
        self.longest_total_wait
    }
}

/**
 * Append the instructions of a statement sequence, patching branch and jump targets.
 */
fn lower(statements: &[Statement], instructions: &mut Vec<Instruction>, count: &mut usize) {
    for statement in statements {
        *count += 1;
        let span = statement.span;
        match &statement.kind {
            StatementKind::Open => instructions.push(Instruction {
                operation: Operation::Open,
                span,
            }),
            StatementKind::Wait(duration) => instructions.push(Instruction {
                operation: Operation::Wait(*duration),
                span,
            }),
            StatementKind::Click(selector) => instructions.push(Instruction {
                operation: Operation::Click(selector.clone()),
                span,
            }),
            StatementKind::If(conditional) => {
                let branch = instructions.len();
                instructions.push(Instruction {
                    operation: Operation::Branch {
                        condition: conditional.condition.clone(),
                        otherwise: 0,
                    },
                    span,
                });
                lower(&conditional.consequence, instructions, count);
                let Some(alternative) = &conditional.alternative else {
                    let target = instructions.len();
                    set_target(&mut instructions[branch], target);
                    continue;
                };
                let jump = instructions.len();
                instructions.push(Instruction {
                    operation: Operation::Jump(0),
                    span,
                });
                let target = instructions.len();
                set_target(&mut instructions[branch], target);
                match alternative {
                    // The nested `if` statement counts itself when it is lowered.
                    Alternative::ElseIf(nested) => {
                        lower(std::slice::from_ref(nested), instructions, count)
                    }
                    Alternative::Else(statements) => lower(statements, instructions, count),
                }
                let target = instructions.len();
                set_target(&mut instructions[jump], target);
            }
        }
    }
}

/**
 * Point a branch or jump instruction at `target`.
 */
fn set_target(instruction: &mut Instruction, target: usize) {
    match &mut instruction.operation {
        Operation::Branch { otherwise, .. } => *otherwise = target,
        Operation::Jump(destination) => *destination = target,
        _ => {}
    }
}

/**
 * Return the longest sum of waits along any path through a statement sequence.
 */
fn longest_wait(statements: &[Statement]) -> Duration {
    statements
        .iter()
        .map(|statement| match &statement.kind {
            StatementKind::Wait(duration) => *duration,
            StatementKind::If(conditional) => {
                let consequence = longest_wait(&conditional.consequence);
                let alternative = match &conditional.alternative {
                    Some(Alternative::ElseIf(nested)) => longest_wait(std::slice::from_ref(nested)),
                    Some(Alternative::Else(statements)) => longest_wait(statements),
                    None => Duration::ZERO,
                };
                consequence.max(alternative)
            }
            StatementKind::Open | StatementKind::Click(_) => Duration::ZERO,
        })
        .fold(Duration::ZERO, Duration::saturating_add)
}

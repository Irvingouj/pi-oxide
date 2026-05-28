//! Rhai script engine for context projection.
//!
//! Scripts receive the tool-result text plus global context variables
//! (turn index, budget, token estimates) so they can make smart,
//! context-aware decisions about what to keep.

use rhai::{Engine, EvalAltResult, Scope};
use tracing::warn;

/// Variables injected into every Rhai script scope.
pub struct ScriptContext {
    pub text: String,
    pub tool_name: String,
    pub tool_call_id: String,
    pub turn_index: usize,
    pub total_turns: usize,
    pub total_tokens: usize,
    pub max_context_tokens: usize,
    pub max_tool_result_chars: usize,
    pub turns_since_compaction: u32,
    pub was_replaced_before: bool,
}

/// Run a Rhai script against a tool result.
///
/// Built-in functions available to scripts:
/// - `head(text, n)` – first N chars
/// - `tail(text, n)` – last N chars
/// - `lines(text)` – split into line array
/// - `join(lines, sep)` – join line array
/// - `contains(text, pattern)` – bool
/// - `regex(text, pattern)` – extract all matches (joined by newline)
/// - `length(text)` – char count
/// - `format(fmt, ...)` – string formatting
///
/// Built-in variables:
/// - `text`, `tool_name`, `tool_call_id`
/// - `turn_index`, `total_turns`
/// - `total_tokens`, `max_context_tokens`
/// - `max_tool_result_chars`, `turns_since_compaction`
/// - `was_replaced_before`
pub fn run_rhai_script(ctx: &ScriptContext, script: &str) -> Result<String, Box<EvalAltResult>> {
    let mut engine = Engine::new();

    // Register built-in text helpers
    engine.register_fn("head", |text: &str, n: i64| -> String {
        text.chars().take(n.max(0) as usize).collect()
    });
    engine.register_fn("tail", |text: &str, n: i64| -> String {
        let count = text.chars().count();
        text.chars()
            .skip(count.saturating_sub(n.max(0) as usize))
            .collect()
    });
    engine.register_fn("lines", |text: &str| -> Vec<rhai::Dynamic> {
        text.lines()
            .map(|s| rhai::Dynamic::from(s.to_string()))
            .collect()
    });
    engine.register_fn("join", |lines: Vec<String>, sep: &str| -> String {
        lines.join(sep)
    });
    engine.register_fn("join", |lines: Vec<rhai::Dynamic>, sep: &str| -> String {
        lines
            .iter()
            .map(|d| d.to_string())
            .collect::<Vec<_>>()
            .join(sep)
    });
    engine.register_fn("contains", |text: &str, pattern: &str| -> bool {
        text.contains(pattern)
    });
    engine.register_fn("length", |text: &str| -> i64 {
        text.chars().count() as i64
    });

    let mut scope = Scope::new();
    scope.push("text", ctx.text.clone());
    scope.push("tool_name", ctx.tool_name.clone());
    scope.push("tool_call_id", ctx.tool_call_id.clone());
    scope.push("turn_index", ctx.turn_index as i64);
    scope.push("total_turns", ctx.total_turns as i64);
    scope.push("total_tokens", ctx.total_tokens as i64);
    scope.push("max_context_tokens", ctx.max_context_tokens as i64);
    scope.push("max_tool_result_chars", ctx.max_tool_result_chars as i64);
    scope.push("turns_since_compaction", ctx.turns_since_compaction as i64);
    scope.push("was_replaced_before", ctx.was_replaced_before);

    engine.eval_with_scope(&mut scope, script)
}

/// Convenience: run script and log + fallback on error.
pub fn apply_rhai_script_or_fallback(
    ctx: &ScriptContext,
    script: &str,
    fallback: impl FnOnce() -> String,
) -> String {
    match run_rhai_script(ctx, script) {
        Ok(result) => result,
        Err(e) => {
            warn!(error = %e, tool_call_id = %ctx.tool_call_id, "rhai script failed; using fallback");
            fallback()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_ctx(text: &str) -> ScriptContext {
        ScriptContext {
            text: text.to_string(),
            tool_name: "test_tool".to_string(),
            tool_call_id: "tc-1".to_string(),
            turn_index: 2,
            total_turns: 5,
            total_tokens: 8000,
            max_context_tokens: 100_000,
            max_tool_result_chars: 50_000,
            turns_since_compaction: 3,
            was_replaced_before: false,
        }
    }

    #[test]
    fn test_head_builtin() {
        let ctx = test_ctx("hello world");
        let result = run_rhai_script(&ctx, r#"head(text, 5)"#).unwrap();
        assert_eq!(result, "hello");
    }

    #[test]
    fn test_tail_builtin() {
        let ctx = test_ctx("hello world");
        let result = run_rhai_script(&ctx, r#"tail(text, 5)"#).unwrap();
        assert_eq!(result, "world");
    }

    #[test]
    fn test_context_variables() {
        let ctx = test_ctx("abc");
        let result =
            run_rhai_script(&ctx, r#"tool_name + "-" + tool_call_id + "-" + turn_index"#).unwrap();
        assert_eq!(result, "test_tool-tc-1-2");
    }

    #[test]
    fn test_lines_and_join() {
        let ctx = test_ctx("a\nb\nc");
        let result = run_rhai_script(&ctx, r#"join(lines(text), "|")"#).unwrap();
        assert_eq!(result, "a|b|c");
    }

    #[test]
    fn test_contains() {
        let ctx = test_ctx("hello world");
        let result = run_rhai_script(
            &ctx,
            r#"if contains(text, "world") { "yes" } else { "no" }"#,
        )
        .unwrap();
        assert_eq!(result, "yes");
    }

    #[test]
    fn test_for_loop() {
        let ctx = test_ctx("abc");
        let result = run_rhai_script(
            &ctx,
            r#"
            let a = [1, 2, 3];
            let b = [];
            for x in a {
                b.push(x);
            }
            join(b, "-")
        "#,
        )
        .unwrap();
        assert_eq!(result, "1-2-3");
    }

    #[test]
    fn test_lines_simple_for() {
        let ctx = test_ctx("a\nb\nc");
        let result = run_rhai_script(
            &ctx,
            r#"
            let all = lines(text);
            let out = [];
            for line in all {
                out.push(line);
            }
            join(out, "|")
        "#,
        )
        .unwrap();
        assert_eq!(result, "a|b|c");
    }

    #[test]
    fn test_lines_and_filter() {
        let ctx = test_ctx("error: foo\ninfo: bar\nerror: baz");
        let script = r#"
            let all = lines(text);
            let errs = [];
            for line in all {
                if contains(line, "error") {
                    errs.push(line);
                }
            }
            join(errs, "\n")
        "#;
        let result = run_rhai_script(&ctx, script).unwrap();
        assert_eq!(result, "error: foo\nerror: baz");
    }

    #[test]
    fn test_smart_budget_decision() {
        let mut ctx = test_ctx("a very long text here");
        ctx.total_tokens = 90_000; // 90% of 100k budget
        let script = r#"
            if total_tokens > max_context_tokens * 8 / 10 {
                head(text, 5)
            } else {
                head(text, 100)
            }
        "#;
        let result = run_rhai_script(&ctx, script).unwrap();
        assert_eq!(result, "a ver");
    }
}

//! Rhai script engine for context projection.
//!
//! Scripts receive the tool-result text plus global context variables
//! (turn index, budget, token estimates) so they can make smart,
//! context-aware decisions about what to keep.

use rhai::{Engine, EvalAltResult, Position, Scope};

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

/// Result of a script evaluation.
#[derive(Debug, Clone, PartialEq)]
pub enum ScriptResult {
    Project { text: String },
    Defer { reevaluate_after: u32 },
}

thread_local! {
    static ENGINE: std::cell::RefCell<Engine> = {
        let mut engine = Engine::new();
        engine.set_max_operations(10_000);

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

        std::cell::RefCell::new(engine)
    };
}

/// Run a Rhai script against a tool result.
///
/// Built-in functions available to scripts:
/// - `head(text, n)` – first N chars
/// - `tail(text, n)` – last N chars
/// - `lines(text)` – split into line array
/// - `join(lines, sep)` – join line array
/// - `contains(text, pattern)` – bool
/// - `length(text)` – char count
///
/// Built-in variables:
/// - `text`, `tool_name`, `tool_call_id`
/// - `turn_index`, `total_turns`
/// - `total_tokens`, `max_context_tokens`
/// - `max_tool_result_chars`, `turns_since_compaction`
/// - `was_replaced_before`
///
/// The script must return a map with an `action` key:
/// - `#{ action: "project", text: "..." }`
/// - `#{ action: "defer", reevaluate_after: 3 }`
pub fn run_rhai_script(ctx: &ScriptContext, script: &str) -> Result<rhai::Map, Box<EvalAltResult>> {
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

    ENGINE.with(|engine| {
        let engine = engine.borrow();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            engine.eval_with_scope(&mut scope, script)
        }));
        match result {
            Ok(val) => val,
            Err(_) => Err(Box::new(EvalAltResult::ErrorRuntime(
                "Rhai script panicked".into(),
                Position::NONE,
            ))),
        }
    })
}

/// Parse a Rhai map into a typed ScriptResult.
pub fn parse_script_result(map: &rhai::Map) -> Result<ScriptResult, String> {
    let action = map
        .get("action")
        .and_then(|d| d.as_immutable_string_ref().ok().map(|s| s.to_string()))
        .ok_or_else(|| "missing action".to_string())?;

    match action.as_str() {
        "project" => {
            let text = map
                .get("text")
                .and_then(|d| d.as_immutable_string_ref().ok().map(|s| s.to_string()))
                .ok_or_else(|| "missing text for project".to_string())?;
            Ok(ScriptResult::Project { text })
        }
        "defer" => {
            let reevaluate_after = map
                .get("reevaluate_after")
                .and_then(|d| d.as_int().ok())
                .ok_or_else(|| "missing reevaluate_after for defer".to_string())?;
            if reevaluate_after < 0 {
                return Err("reevaluate_after must be non-negative".to_string());
            }
            if reevaluate_after > u32::MAX as i64 {
                return Err("reevaluate_after too large".to_string());
            }
            Ok(ScriptResult::Defer {
                reevaluate_after: reevaluate_after as u32,
            })
        }
        other => Err(format!("unknown action: {}", other)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tracing::warn;

    /// Convenience: run script and log + fallback on error.
    fn apply_rhai_script_or_fallback(
        ctx: &ScriptContext,
        script: &str,
        fallback: impl FnOnce() -> ScriptResult,
    ) -> ScriptResult {
        match run_rhai_script(ctx, script) {
            Ok(map) => match parse_script_result(&map) {
                Ok(result) => result,
                Err(e) => {
                    warn!(
                        error = %e,
                        tool_call_id = %ctx.tool_call_id,
                        "rhai script returned invalid map; using fallback"
                    );
                    fallback()
                }
            },
            Err(e) => {
                warn!(error = %e, tool_call_id = %ctx.tool_call_id, "rhai script failed; using fallback");
                fallback()
            }
        }
    }

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
        let result =
            run_rhai_script(&ctx, r#"#{ action: "project", text: head(text, 5) }"#).unwrap();
        let parsed = parse_script_result(&result).unwrap();
        assert_eq!(
            parsed,
            ScriptResult::Project {
                text: "hello".to_string()
            }
        );
    }

    #[test]
    fn test_tail_builtin() {
        let ctx = test_ctx("hello world");
        let result =
            run_rhai_script(&ctx, r#"#{ action: "project", text: tail(text, 5) }"#).unwrap();
        let parsed = parse_script_result(&result).unwrap();
        assert_eq!(
            parsed,
            ScriptResult::Project {
                text: "world".to_string()
            }
        );
    }

    #[test]
    fn test_context_variables() {
        let ctx = test_ctx("abc");
        let result = run_rhai_script(
            &ctx,
            r#"#{ action: "project", text: tool_name + "-" + tool_call_id + "-" + turn_index }"#,
        )
        .unwrap();
        let parsed = parse_script_result(&result).unwrap();
        assert_eq!(
            parsed,
            ScriptResult::Project {
                text: "test_tool-tc-1-2".to_string(),
            }
        );
    }

    #[test]
    fn test_lines_and_join() {
        let ctx = test_ctx("a\nb\nc");
        let result = run_rhai_script(
            &ctx,
            r#"#{ action: "project", text: join(lines(text), "|") }"#,
        )
        .unwrap();
        let parsed = parse_script_result(&result).unwrap();
        assert_eq!(
            parsed,
            ScriptResult::Project {
                text: "a|b|c".to_string(),
            }
        );
    }

    #[test]
    fn test_contains() {
        let ctx = test_ctx("hello world");
        let result = run_rhai_script(
            &ctx,
            r#"if contains(text, "world") { #{ action: "project", text: "yes" } } else { #{ action: "project", text: "no" } }"#,
        )
        .unwrap();
        let parsed = parse_script_result(&result).unwrap();
        assert_eq!(
            parsed,
            ScriptResult::Project {
                text: "yes".to_string(),
            }
        );
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
            #{ action: "project", text: join(b, "-") }
        "#,
        )
        .unwrap();
        let parsed = parse_script_result(&result).unwrap();
        assert_eq!(
            parsed,
            ScriptResult::Project {
                text: "1-2-3".to_string(),
            }
        );
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
            #{ action: "project", text: join(out, "|") }
        "#,
        )
        .unwrap();
        let parsed = parse_script_result(&result).unwrap();
        assert_eq!(
            parsed,
            ScriptResult::Project {
                text: "a|b|c".to_string(),
            }
        );
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
            #{ action: "project", text: join(errs, "\n") }
        "#;
        let result = run_rhai_script(&ctx, script).unwrap();
        let parsed = parse_script_result(&result).unwrap();
        assert_eq!(
            parsed,
            ScriptResult::Project {
                text: "error: foo\nerror: baz".to_string(),
            }
        );
    }

    #[test]
    fn test_smart_budget_decision() {
        let mut ctx = test_ctx("a very long text here");
        ctx.total_tokens = 90_000; // 90% of 100k budget
        let script = r#"
            if total_tokens > max_context_tokens * 8 / 10 {
                #{ action: "project", text: head(text, 5) }
            } else {
                #{ action: "project", text: head(text, 100) }
            }
        "#;
        let result = run_rhai_script(&ctx, script).unwrap();
        let parsed = parse_script_result(&result).unwrap();
        assert_eq!(
            parsed,
            ScriptResult::Project {
                text: "a ver".to_string(),
            }
        );
    }

    #[test]
    fn test_defer_action() {
        let ctx = test_ctx("hello world");
        let result = run_rhai_script(&ctx, r#"#{ action: "defer", reevaluate_after: 3 }"#).unwrap();
        let parsed = parse_script_result(&result).unwrap();
        assert_eq!(
            parsed,
            ScriptResult::Defer {
                reevaluate_after: 3
            }
        );
    }

    #[test]
    fn test_fallback_on_bad_script() {
        let ctx = test_ctx("hello world");
        let result =
            apply_rhai_script_or_fallback(&ctx, "bad_syntax!!!", || ScriptResult::Project {
                text: "fallback".to_string(),
            });
        assert_eq!(
            result,
            ScriptResult::Project {
                text: "fallback".to_string(),
            }
        );
    }

    #[test]
    fn parse_missing_action() {
        let mut map = rhai::Map::new();
        map.insert("text".into(), "hello".into());
        let err = parse_script_result(&map).unwrap_err();
        assert_eq!(err, "missing action");
    }

    #[test]
    fn parse_unknown_action() {
        let mut map = rhai::Map::new();
        map.insert("action".into(), "unknown".into());
        let err = parse_script_result(&map).unwrap_err();
        assert_eq!(err, "unknown action: unknown");
    }

    #[test]
    fn parse_missing_text_for_project() {
        let mut map = rhai::Map::new();
        map.insert("action".into(), "project".into());
        let err = parse_script_result(&map).unwrap_err();
        assert_eq!(err, "missing text for project");
    }

    #[test]
    fn parse_missing_reevaluate_after_for_defer() {
        let mut map = rhai::Map::new();
        map.insert("action".into(), "defer".into());
        let err = parse_script_result(&map).unwrap_err();
        assert_eq!(err, "missing reevaluate_after for defer");
    }

    #[test]
    fn test_negative_reevaluate_after_rejected() {
        let mut map = rhai::Map::new();
        map.insert("action".into(), "defer".into());
        map.insert("reevaluate_after".into(), rhai::Dynamic::from(-1 as i64));
        let err = parse_script_result(&map).unwrap_err();
        assert_eq!(err, "reevaluate_after must be non-negative");
    }

    #[test]
    fn test_max_operations_limit_kills_infinite_loop() {
        let ctx = test_ctx("hello world");
        let script = r#"
            let i = 0;
            while true {
                i = i + 1;
            }
            #{ action: "project", text: "never" }
        "#;
        let result = run_rhai_script(&ctx, script);
        assert!(
            result.is_err(),
            "infinite loop should exceed max_operations limit and error"
        );
    }
}

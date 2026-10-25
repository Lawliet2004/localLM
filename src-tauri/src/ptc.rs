//! Bounded executable plan for `ptc_run`: a strict declarative JSON AST, never
//! host-evaluated code. A plan is `{"body": [...], "return": <expr>}`; every
//! nested tool call goes through the same offered-alias lookup, capability
//! recheck, argument validation, per-call approval, audit row, timeout,
//! envelope normalization and bounding as a direct model call.
//!
//! Statements: `let` bindings, `call` (tool RPC), `if`, bounded `for`,
//! `parallel` (explicit independent trusted reads only). Expressions: JSON
//! literals, `{"$": "name", "path": [...]}` references, and a fixed set of
//! bounded `{"op": ...}` transforms. There is no recursion, no model-written
//! authorization field, and no implicit retry — a failed, cancelled or
//! uncertain-outcome call aborts the plan and reports completed calls plus
//! the unresolved statement indices.

use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::time::Instant;

use crate::harness::{HarnessCtx, HarnessOutcome};

// Execution budgets. Tool calls keep the historical 8-step cap; all other
// limits make a plan terminate regardless of the data it carries.
const MAX_PLAN_NODES: usize = 256;
/// Nested `if`/`for`/`parallel` bodies — expression nesting has its own cap.
const MAX_PLAN_DEPTH: usize = 8;
/// Raw JSON depth rejected at parse, ahead of any recursive walker.
const MAX_JSON_DEPTH: usize = 32;
const MAX_STATEMENTS: usize = 128;
const MAX_ITERATIONS: usize = 64;
const MAX_CALLS: usize = 8;
const MAX_PLAN_SECS: u64 = 60;
const MAX_VARS: usize = 32;
const MAX_VAR_BYTES: usize = 65_536;
const MAX_EXPR_DEPTH: usize = 32;

/// Object keys a plan may never construct — they mean nothing to serde_json
/// but are prototype-pollution vectors for any JS consumer downstream.
const FORBIDDEN_KEYS: [&str; 3] = ["__proto__", "prototype", "constructor"];

/// Top-level plan keys that could look like authority. A plan may only ask
/// for calls — the host dispatcher owns every permission decision.
const FORBIDDEN_ROOT_KEYS: [&str; 4] = ["allow", "allowed", "permissions", "approval"];

#[derive(Debug)]
struct Abort {
    status: &'static str,
    message: String,
}
type Step<T> = Result<T, Abort>;

/// Why one `parallel` member settled badly — keeps the timeout cause typed
/// instead of sniffing message text downstream.
enum MemberError {
    Cancelled,
    TimedOut,
    Failed(String),
}

fn error(message: impl Into<String>) -> Abort {
    Abort {
        status: "error",
        message: message.into(),
    }
}

fn forbidden_key(key: &str) -> bool {
    FORBIDDEN_KEYS.contains(&key)
}

/// Count AST nodes and verify nesting/statement caps before anything runs.
fn validate_node(value: &Value, depth: usize, nodes: &mut usize) -> Step<()> {
    if depth > MAX_JSON_DEPTH {
        return Err(error(format!(
            "Plan JSON nesting exceeds {MAX_JSON_DEPTH}."
        )));
    }
    *nodes += 1;
    if *nodes > MAX_PLAN_NODES {
        return Err(error(format!("Plan exceeds {MAX_PLAN_NODES} nodes.")));
    }
    match value {
        Value::Array(items) => {
            for item in items {
                validate_node(item, depth + 1, nodes)?;
            }
        }
        Value::Object(map) => {
            for (key, item) in map {
                if forbidden_key(key) {
                    return Err(error(format!("Plan object key '{key}' is not allowed.")));
                }
                validate_node(item, depth + 1, nodes)?;
            }
        }
        _ => {}
    }
    Ok(())
}

struct Exec<'a> {
    ctx: &'a HarnessCtx<'a>,
    scopes: Vec<HashMap<String, Value>>,
    /// Audit entries in dispatch order — one per attempted tool call.
    calls: Vec<Value>,
    statements: usize,
    iterations: usize,
    tool_calls: usize,
    /// Top-level statements attempted — the unresolved tail is
    /// `top_attempted..body.len()` when a plan aborts mid-run. A statement
    /// that starts counts as attempted even when its call aborts the plan.
    top_attempted: usize,
    /// Real events surfaced by nested harness calls, propagated to the
    /// outcome in member order instead of being silently dropped.
    events: Vec<crate::harness::PendingEvent>,
    /// Per-execution nonce — nested call audit IDs stay unique across
    /// `parallel` blocks and repeated `ptc_run` invocations in one turn.
    nonce: String,
    deadline: Instant,
    cancel: tokio::sync::watch::Receiver<bool>,
}

impl<'a> Exec<'a> {
    fn new(ctx: &'a HarnessCtx<'a>) -> Self {
        Self {
            ctx,
            scopes: vec![HashMap::new()],
            calls: Vec::new(),
            statements: 0,
            iterations: 0,
            tool_calls: 0,
            top_attempted: 0,
            events: Vec::new(),
            nonce: uuid::Uuid::new_v4().simple().to_string()[..8].to_string(),
            deadline: Instant::now() + std::time::Duration::from_secs(MAX_PLAN_SECS),
            cancel: ctx.state.cancel.subscribe(),
        }
    }

    /// The run's shared non-retryable signature set. Created on demand so
    /// replay protection holds for every dispatch path, not just turns the
    /// parent loop registered — the parent installs its own set first when
    /// present, so both share the same cell.
    fn turn_blocked(
        &self,
    ) -> Option<std::sync::Arc<std::sync::Mutex<std::collections::HashSet<String>>>> {
        let mut map = self.ctx.state.turn_blocked.lock().ok()?;
        Some(
            map.entry(self.ctx.run_id.clone())
                .or_insert_with(Default::default)
                .clone(),
        )
    }

    /// Record a signature whose outcome must not replay this turn. Called on
    /// every non-success outcome — uncertain writes, failures, denials and
    /// cancellations are all unsafe to silently re-dispatch.
    fn block_signature(&self, signature: &str) {
        if let Some(blocked) = self.turn_blocked() {
            if let Ok(mut set) = blocked.lock() {
                set.insert(signature.to_string());
            }
        }
    }

    /// Race one future against generation cancellation and the plan deadline.
    /// `changed()` alone misses a cancel the executor already observed, so the
    /// stored flag is checked first — identical to the sequential `guard()`.
    async fn await_bounded<F: std::future::Future>(&mut self, future: F) -> Step<F::Output> {
        if *self.cancel.borrow() {
            return Err(Abort {
                status: "cancelled",
                message: "Plan cancelled.".into(),
            });
        }
        let mut cancel = self.cancel.clone();
        let deadline = tokio::time::Instant::from_std(self.deadline);
        tokio::select! {
            _ = cancel.changed() => Err(Abort { status: "cancelled", message: "Plan cancelled.".into() }),
            _ = tokio::time::sleep_until(deadline) => Err(error(format!("Plan exceeded its {MAX_PLAN_SECS}s time budget."))),
            output = future => Ok(output),
        }
    }

    fn guard(&mut self) -> Step<()> {
        if *self.cancel.borrow() {
            return Err(Abort {
                status: "cancelled",
                message: "Plan cancelled.".into(),
            });
        }
        if Instant::now() >= self.deadline {
            return Err(Abort {
                status: "error",
                message: format!("Plan exceeded its {MAX_PLAN_SECS}s time budget."),
            });
        }
        Ok(())
    }

    fn lookup(&self, name: &str) -> Step<&Value> {
        for scope in self.scopes.iter().rev() {
            if let Some(value) = scope.get(name) {
                return Ok(value);
            }
        }
        Err(error(format!("Missing variable '{name}'.")))
    }

    fn bind(&mut self, name: &str, value: Value) -> Step<()> {
        if name.is_empty()
            || name.len() > 64
            || !name
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
        {
            return Err(error(format!("Invalid variable name '{name}'.")));
        }
        if forbidden_key(name) {
            return Err(error(format!("Variable name '{name}' is not allowed.")));
        }
        // Plan-global bindings: `let`/`as` update the nearest existing scope
        // (so a loop body can accumulate), or introduce the name at the ROOT
        // scope — a binding never dies with an if/for block. The only scoped
        // names are `for` vars and map/filter `as` items, bound via bind_local.
        let index = self
            .scopes
            .iter()
            .rposition(|scope| scope.contains_key(name))
            .unwrap_or(0);
        self.scopes[index].insert(name.to_string(), value);
        let total: usize = self.scopes.iter().map(|scope| scope.len()).sum();
        if total > MAX_VARS {
            return Err(error(format!("Plan exceeds {MAX_VARS} variables.")));
        }
        let bytes: usize = self
            .scopes
            .iter()
            .flat_map(|scope| scope.values())
            .map(|value| value.to_string().len())
            .sum();
        if bytes > MAX_VAR_BYTES {
            return Err(error("Plan variables exceed 64 KiB."));
        }
        Ok(())
    }

    /// Innermost-scope binding for genuinely scoped names: `for` vars and
    /// map/filter item aliases. Dies with the block that created it.
    fn bind_local(&mut self, name: &str, value: Value) -> Step<()> {
        if name.is_empty()
            || name.len() > 64
            || !name
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
        {
            return Err(error(format!("Invalid variable name '{name}'.")));
        }
        if forbidden_key(name) {
            return Err(error(format!("Variable name '{name}' is not allowed.")));
        }
        self.scopes
            .last_mut()
            .expect("a scope is pushed")
            .insert(name.to_string(), value);
        Ok(())
    }

    fn eval(&mut self, expr: &Value, depth: usize) -> Step<Value> {
        if depth > MAX_EXPR_DEPTH {
            return Err(error(format!(
                "Expression nesting exceeds {MAX_EXPR_DEPTH}."
            )));
        }
        // Array literals evaluate element-wise — `[{"$":"x"}, 1]` resolves.
        if let Value::Array(items) = expr {
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                out.push(self.eval(item, depth + 1)?);
            }
            return Ok(json!(out));
        }
        let Value::Object(map) = expr else {
            return Ok(expr.clone());
        };
        if let Some(name) = map.get("$").and_then(Value::as_str) {
            let mut value = self.lookup(name)?.clone();
            if let Some(path) = map.get("path").and_then(Value::as_array) {
                value = self.resolve_path(value, path, name)?;
            }
            return Ok(value);
        }
        if let Some(op) = map.get("op").and_then(Value::as_str) {
            return self.eval_op(op, map, depth);
        }
        // Plain object literal — every value is itself an expression.
        let mut out = Map::new();
        for (key, item) in map {
            if forbidden_key(key) {
                return Err(error(format!("Object key '{key}' is not allowed.")));
            }
            out.insert(key.clone(), self.eval(item, depth + 1)?);
        }
        Ok(Value::Object(out))
    }

    fn resolve_path(&self, mut value: Value, path: &[Value], name: &str) -> Step<Value> {
        let mut walked = String::new();
        for segment in path {
            walked.push('/');
            match segment {
                Value::String(key) => {
                    walked.push_str(key);
                    value = value
                        .get(key)
                        .cloned()
                        .ok_or_else(|| error(format!("Path '{name}{walked}' is not set.")))?;
                }
                Value::Number(index) => {
                    let Some(index) = index.as_u64() else {
                        return Err(error(format!(
                            "Path '{name}{walked}' needs a non-negative index."
                        )));
                    };
                    walked.push_str(&index.to_string());
                    value = value
                        .get(index as usize)
                        .cloned()
                        .ok_or_else(|| error(format!("Path '{name}{walked}' is not set.")))?;
                }
                _ => {
                    return Err(error(format!(
                        "Path '{name}' segments must be keys or indices."
                    )))
                }
            }
        }
        Ok(value)
    }

    fn eval_op(&mut self, op: &str, map: &Map<String, Value>, depth: usize) -> Step<Value> {
        let arg = |key: &str| {
            map.get(key)
                .ok_or_else(|| error(format!("op '{op}' needs '{key}'.")))
        };
        match op {
            "eq" | "ne" | "lt" | "le" | "gt" | "ge" => {
                let a = self.eval(arg("a")?, depth + 1)?;
                let b = self.eval(arg("b")?, depth + 1)?;
                let ordering = compare(&a, &b).ok_or_else(|| {
                    error(format!(
                        "op '{op}' cannot compare {} with {}.",
                        kind(&a),
                        kind(&b)
                    ))
                })?;
                let result = match op {
                    "eq" => ordering == std::cmp::Ordering::Equal,
                    "ne" => ordering != std::cmp::Ordering::Equal,
                    "lt" => ordering == std::cmp::Ordering::Less,
                    "le" => ordering != std::cmp::Ordering::Greater,
                    "gt" => ordering == std::cmp::Ordering::Greater,
                    _ => ordering != std::cmp::Ordering::Less,
                };
                Ok(json!(result))
            }
            "not" => Ok(json!(!truthy(&self.eval(arg("value")?, depth + 1)?))),
            "and" | "or" => {
                let args = arg("args")?
                    .as_array()
                    .ok_or_else(|| error(format!("op '{op}' needs an 'args' array.")))?;
                let mut result = op == "and";
                for item in args {
                    let value = truthy(&self.eval(item, depth + 1)?);
                    result = if op == "and" {
                        result && value
                    } else {
                        result || value
                    };
                }
                Ok(json!(result))
            }
            "get" => {
                let value = self.eval(arg("of")?, depth + 1)?;
                let path = arg("path")?
                    .as_array()
                    .ok_or_else(|| error("op 'get' needs a 'path' array."))?;
                self.resolve_path(value, path, "value")
            }
            "len" => {
                let value = self.eval(arg("of")?, depth + 1)?;
                let len = match &value {
                    Value::Array(items) => items.len(),
                    Value::Object(map) => map.len(),
                    Value::String(text) => text.chars().count(),
                    _ => {
                        return Err(error(format!(
                            "op 'len' does not apply to {}.",
                            kind(&value)
                        )))
                    }
                };
                Ok(json!(len))
            }
            "keys" | "values" => {
                let value = self.eval(arg("of")?, depth + 1)?;
                let Value::Object(map) = value else {
                    return Err(error(format!("op '{op}' needs an object.")));
                };
                if op == "keys" {
                    Ok(json!(map.keys().cloned().collect::<Vec<_>>()))
                } else {
                    Ok(json!(map.values().cloned().collect::<Vec<_>>()))
                }
            }
            "first" | "last" => {
                let value = self.eval(arg("of")?, depth + 1)?;
                let items = value
                    .as_array()
                    .ok_or_else(|| error(format!("op '{op}' needs an array.")))?;
                let picked = if op == "first" {
                    items.first()
                } else {
                    items.last()
                };
                picked
                    .cloned()
                    .ok_or_else(|| error(format!("op '{op}' on an empty array.")))
            }
            "concat" => {
                let args = arg("args")?
                    .as_array()
                    .ok_or_else(|| error("op 'concat' needs an 'args' array."))?;
                let mut values = Vec::new();
                for item in args {
                    values.push(self.eval(item, depth + 1)?);
                }
                if values.iter().all(|value| value.is_array()) {
                    let mut out = Vec::new();
                    for value in values {
                        out.extend(value.as_array().expect("array").iter().cloned());
                    }
                    return Ok(json!(out));
                }
                if values.iter().all(|value| value.is_string()) {
                    let mut out = String::new();
                    for value in values {
                        out.push_str(value.as_str().expect("string"));
                    }
                    if out.len() > MAX_VAR_BYTES {
                        return Err(error("op 'concat' exceeds 64 KiB."));
                    }
                    return Ok(json!(out));
                }
                Err(error("op 'concat' needs all-arrays or all-strings."))
            }
            "join" => {
                let list = self.eval(arg("list")?, depth + 1)?;
                let sep = self.eval(arg("sep")?, depth + 1)?;
                let sep = sep
                    .as_str()
                    .ok_or_else(|| error("op 'join' needs a string 'sep'."))?;
                let items = list
                    .as_array()
                    .ok_or_else(|| error("op 'join' needs a 'list' array."))?;
                let parts: Vec<String> = items
                    .iter()
                    .map(|item| match item {
                        Value::String(text) => Ok(text.clone()),
                        Value::Number(_) | Value::Bool(_) => Ok(item.to_string()),
                        _ => Err(error("op 'join' items must be strings or scalars.")),
                    })
                    .collect::<Step<_>>()?;
                Ok(json!(parts.join(sep)))
            }
            "contains" => {
                let of = self.eval(arg("of")?, depth + 1)?;
                let item = self.eval(arg("item")?, depth + 1)?;
                let result = match &of {
                    Value::Array(items) => items
                        .iter()
                        .any(|entry| compare(entry, &item) == Some(std::cmp::Ordering::Equal)),
                    Value::String(text) => item
                        .as_str()
                        .map(|needle| text.contains(needle))
                        .unwrap_or(false),
                    _ => {
                        return Err(error(format!(
                            "op 'contains' does not apply to {}.",
                            kind(&of)
                        )))
                    }
                };
                Ok(json!(result))
            }
            "merge" => {
                let args = arg("args")?
                    .as_array()
                    .ok_or_else(|| error("op 'merge' needs an 'args' array."))?;
                let mut out = Map::new();
                for item in args {
                    let value = self.eval(item, depth + 1)?;
                    let Value::Object(source) = value else {
                        return Err(error("op 'merge' members must be objects."));
                    };
                    for (key, member) in source {
                        if forbidden_key(&key) {
                            return Err(error(format!(
                                "op 'merge' source key '{key}' is not allowed."
                            )));
                        }
                        out.insert(key, member);
                    }
                }
                Ok(Value::Object(out))
            }
            "map" | "filter" => {
                let list = self.eval(arg("list")?, depth + 1)?;
                let items = list
                    .as_array()
                    .ok_or_else(|| error(format!("op '{op}' needs a 'list' array.")))?;
                let var = map.get("as").and_then(Value::as_str).unwrap_or("item");
                let body = if op == "map" {
                    arg("select")?
                } else {
                    arg("where")?
                };
                let mut out = Vec::new();
                for item in items {
                    self.iterations += 1;
                    if self.iterations > MAX_ITERATIONS {
                        return Err(error(format!("Plan exceeds {MAX_ITERATIONS} iterations.")));
                    }
                    self.guard()?;
                    self.scopes.push(HashMap::new());
                    self.bind_local(var, item.clone())?;
                    let selected = self.eval(body, depth + 1);
                    self.scopes.pop();
                    match op {
                        "map" => out.push(selected?),
                        _ => {
                            if truthy(&selected?) {
                                out.push(item.clone());
                            }
                        }
                    }
                }
                Ok(json!(out))
            }
            "str" => {
                let value = self.eval(arg("of")?, depth + 1)?;
                Ok(match value {
                    Value::String(_) => value,
                    other => json!(other.to_string()),
                })
            }
            _ => Err(error(format!("Unknown plan op '{op}'."))),
        }
    }

    async fn run_body(&mut self, body: &[Value], depth: usize) -> Step<()> {
        self.scopes.push(HashMap::new());
        let result = self.run_statements(body, depth, false).await;
        self.scopes.pop();
        result
    }

    fn run_statements<'b>(
        &'b mut self,
        body: &'b [Value],
        depth: usize,
        top: bool,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Step<()>> + Send + 'b>> {
        // Boxed because statement forms recurse through if/for bodies.
        Box::pin(async move {
            for (index, statement) in body.iter().enumerate() {
                self.guard()?;
                self.statements += 1;
                if self.statements > MAX_STATEMENTS {
                    return Err(error(format!("Plan exceeds {MAX_STATEMENTS} statements.")));
                }
                if top {
                    self.top_attempted = index + 1;
                }
                self.run_statement(statement, depth).await?;
            }
            Ok(())
        })
    }

    async fn run_statement(&mut self, statement: &Value, depth: usize) -> Step<()> {
        if depth >= MAX_PLAN_DEPTH {
            return Err(error(format!("Plan nesting exceeds {MAX_PLAN_DEPTH}.")));
        }
        let Value::Object(map) = statement else {
            return Err(error("Plan statements must be objects."));
        };
        if map.len() != 1 {
            return Err(error("A plan statement has exactly one form key."));
        }
        let (form, spec) = map.iter().next().expect("nonempty");
        match form.as_str() {
            "let" => {
                let name = spec
                    .get("name")
                    .and_then(Value::as_str)
                    .ok_or_else(|| error("'let' needs a 'name'."))?;
                let name = name.to_string();
                let value = spec
                    .get("value")
                    .ok_or_else(|| error("'let' needs a 'value'."))?;
                let value = self.eval(value, 0)?;
                self.bind(&name, value)
            }
            "call" => self.run_call(spec).await,
            "if" => {
                let cond = spec
                    .get("cond")
                    .ok_or_else(|| error("'if' needs 'cond'."))?;
                let cond = self.eval(cond, 0)?;
                let Value::Bool(cond) = cond else {
                    return Err(error("'if' cond must evaluate to a boolean."));
                };
                let branch = if cond { "then" } else { "else" };
                // A present-but-malformed branch is a plan defect — it must
                // fail loudly, never degrade into a silently empty block.
                let body = match spec.get(branch) {
                    Some(value) => value.as_array().cloned().ok_or_else(|| {
                        error(format!("'if' '{branch}' must be a statement array."))
                    })?,
                    None => Vec::new(),
                };
                self.run_body(&body, depth + 1).await
            }
            "for" => {
                let var = spec
                    .get("var")
                    .and_then(Value::as_str)
                    .ok_or_else(|| error("'for' needs 'var'."))?
                    .to_string();
                let list = spec.get("in").ok_or_else(|| error("'for' needs 'in'."))?;
                let list = self.eval(list, 0)?;
                let items = list
                    .as_array()
                    .ok_or_else(|| error("'for' needs an array 'in'."))?
                    .clone();
                let cap = spec
                    .get("max")
                    .and_then(Value::as_u64)
                    .unwrap_or(MAX_ITERATIONS as u64) as usize;
                let body = match spec.get("body") {
                    Some(value) => value
                        .as_array()
                        .cloned()
                        .ok_or_else(|| error("'for' 'body' must be a statement array."))?,
                    None => Vec::new(),
                };
                for (index, item) in items.iter().enumerate() {
                    if index >= cap {
                        return Err(error(format!("'for' exceeded its {cap}-iteration cap.")));
                    }
                    self.iterations += 1;
                    if self.iterations > MAX_ITERATIONS {
                        return Err(error(format!("Plan exceeds {MAX_ITERATIONS} iterations.")));
                    }
                    self.guard()?;
                    self.scopes.push(HashMap::new());
                    self.bind_local(&var, item.clone())?;
                    let result = self.run_statements(&body, depth + 1, false).await;
                    self.scopes.pop();
                    result?;
                }
                Ok(())
            }
            "parallel" => {
                let body = spec
                    .get("body")
                    .and_then(Value::as_array)
                    .ok_or_else(|| error("'parallel' needs a 'body' array of call statements."))?;
                self.run_parallel(body).await
            }
            other => Err(error(format!("Unknown plan statement '{other}'."))),
        }
    }

    /// Resolve args and run one nested tool through the real dispatch gates.
    /// `automatic_only` rejects anything that would show a dialog — the
    /// `parallel` form can never block on approvals mid-group.
    async fn dispatch_call(
        &mut self,
        tool_name: &str,
        tool_args: Value,
        call_id: &str,
        automatic_only: bool,
    ) -> Step<(Value, bool)> {
        if matches!(tool_name, "ptc_run" | "finish" | "update_context") {
            return Err(error(format!(
                "Tool '{tool_name}' cannot run inside a plan."
            )));
        }
        // `tool_invoke` is a discovery wrapper: it names the real tool in its
        // arguments. Resolve it to that tool first, exactly as the parent
        // chat loop does, so the documented "search then invoke" pairing
        // works inside a plan. Dispatching the wrapper itself is refused by
        // the harness.
        let (tool, tool_args) = if tool_name == crate::tool_discovery::INVOKE {
            let (target, resolved) =
                crate::tool_discovery::resolve(self.ctx.inherit_tools.iter(), tool_name, tool_args)
                    .map_err(|error_message| {
                        error(format!(
                            "Tool '{tool_name}' rejected its arguments: {error_message}"
                        ))
                    })?;
            (target, resolved)
        } else {
            let Some(tool) = self
                .ctx
                .inherit_tools
                .iter()
                .find(|tool| tool.alias == tool_name)
            else {
                return Err(error(format!(
                    "Tool '{tool_name}' is not offered in this preset."
                )));
            };
            let tool_args = tool.prepare_arguments(tool_args).map_err(|error_message| {
                error(format!(
                    "Tool '{tool_name}' rejected its arguments: {error_message}"
                ))
            })?;
            (tool, tool_args)
        };
        if let Some(reason) = crate::capabilities::tool_capability(tool).and_then(|capability| {
            crate::capabilities::ToolPolicy::guard(self.ctx.state.database()).denial(capability)
        }) {
            return Err(Abort {
                status: "denied",
                message: reason,
            });
        }
        let tool_args = tool.prepare_arguments(tool_args).map_err(|error_message| {
            error(format!(
                "Tool '{tool_name}' rejected its arguments: {error_message}"
            ))
        })?;
        // Turn-scoped replay protection: a signature the turn already marked
        // non-retryable (uncertain, failed or denied) is refused before any
        // approval prompt or dispatch — a second plan cannot bypass it.
        let signature = crate::continuation::tool_signature(tool, &tool_args);
        {
            let blocked = self.turn_blocked().ok_or_else(|| {
                error("The turn replay-protection registry is unavailable; no tool was dispatched.")
            })?;
            if blocked
                .lock()
                .map(|set| set.contains(&signature))
                .unwrap_or(true)
            {
                let denied = crate::tool_envelope::failure_notice(
                    "denied",
                    "Not repeated: an earlier attempt this turn had an uncertain outcome.",
                    false,
                    crate::tool_envelope::SIDE_UNKNOWN,
                    &json!({"blocked": true}),
                );
                let audit = json!({"connector": tool.connector, "name": tool.tool.name, "arguments": tool_args,
                    "decision": "denied", "accessMode": self.ctx.access_mode,
                    "authorization": "turn replay protection",
                    "program": true, "callId": call_id, "stepId": self.ctx.step_id});
                if let Ok(store) = self.ctx.state.database() {
                    let _ = store.append_message(
                        &self.ctx.conversation_id,
                        "tool",
                        &json!({"request": audit, "result": denied}).to_string(),
                        "complete",
                    );
                }
                self.calls
                    .push(json!({"tool": tool_name, "status": "denied", "callId": call_id}));
                return Err(Abort {
                    status: "denied",
                    message: "Not repeated: an earlier attempt this turn had an uncertain outcome."
                        .into(),
                });
            }
        }
        let automatic = !tool.requires_approval(&tool_args)
            && self
                .ctx
                .access_mode
                .automatic_reason(tool.trusted_read())
                .is_some();
        if automatic_only && !automatic {
            return Err(error(format!(
                "Tool '{tool_name}' needs approval and cannot join a 'parallel' block."
            )));
        }
        if *self.cancel.borrow() {
            return Err(Abort {
                status: "cancelled",
                message: "Plan cancelled.".into(),
            });
        }
        let allow = if automatic {
            true
        } else {
            let (approval_id, decision) = self.ctx.state.approvals.request().map_err(error)?;
            let _pending = self.ctx.state.approvals.guard(&approval_id);
            (self.ctx.emit)(
                format!("Awaiting approval: {tool_name} (plan)"),
                Some(
                    json!({"id": approval_id, "connector": tool.connector, "name": tool.tool.name, "arguments": tool_args}),
                ),
            );
            // Cancellation and the plan deadline interrupt the approval wait —
            // a plan never waits out its budget on a dialog.
            let allow = self
                .await_bounded(tokio::time::timeout(
                    std::time::Duration::from_secs(600),
                    decision,
                ))
                .await?
                .map_err(|_| error("Approval timed out."))?
                .map_err(|_| error("Approval was withdrawn."))?;
            self.ctx.state.approvals.remove(&approval_id);
            (self.ctx.emit)("Approval decision recorded".into(), Some(Value::Null));
            allow
        };
        let audit = json!({"connector": tool.connector, "name": tool.tool.name, "arguments": tool_args,
            "decision": if allow { "allowed" } else { "denied" }, "accessMode": self.ctx.access_mode,
            "authorization": if automatic { "auto-approved plan call" } else { "user approval decision" },
            "program": true, "callId": call_id, "stepId": self.ctx.step_id});
        let row = {
            let store = self.ctx.state.database().map_err(error)?;
            store
                .append_message(
                    &self.ctx.conversation_id,
                    "tool",
                    &audit.to_string(),
                    if allow { "streaming" } else { "complete" },
                )
                .map_err(error)?
        };
        if !allow {
            let denied = crate::tool_envelope::failure_notice(
                "denied",
                "The user denied this plan call. The plan aborted.",
                false,
                crate::tool_envelope::SIDE_NONE,
                &json!({"denied": true}),
            );
            if let Ok(store) = self.ctx.state.database() {
                let _ = store.update_message(
                    &row.id,
                    &json!({"request": audit, "result": denied}).to_string(),
                    "",
                    "complete",
                );
            }
            self.calls
                .push(json!({"tool": tool_name, "status": "denied", "callId": call_id}));
            self.block_signature(&signature);
            return Err(Abort {
                status: "denied",
                message: "The user denied a plan call.".into(),
            });
        }
        // Shared turn budgets gate *before* dispatch: a denied approval above
        // already returned without spending, and a refused charge here must
        // settle the opened audit row instead of leaving it streaming.
        if let Err(message) =
            crate::continuation::admit_tool(self.ctx.state, &self.ctx.run_id, tool_name, &tool_args)
        {
            let denied = crate::tool_envelope::failure_notice(
                "tool-budget-blocked",
                &message,
                false,
                crate::tool_envelope::SIDE_NONE,
                &json!({"budget":true}),
            );
            if let Ok(store) = self.ctx.state.database() {
                let _ = store.update_message(
                    &row.id,
                    &json!({"request":audit,"result":denied}).to_string(),
                    "",
                    "complete",
                );
            }
            self.calls
                .push(json!({"tool":tool_name,"status":"denied","callId":call_id}));
            return Err(Abort {
                status: "denied",
                message,
            });
        }
        // The audit row above is already durable — an interrupt mid-call must
        // settle it to the typed outcome, never leave it 'streaming' forever.
        let interrupted = |abort: &Abort| -> Value {
            let cause = crate::tool_envelope::Cause {
                cancelled: abort.status == "cancelled",
                timeout: abort.status != "cancelled",
                ..Default::default()
            };
            crate::tool_envelope::normalize_error(&abort.message, cause, tool.envelope_family())
        };
        let outcome: Value = if tool.is_harness() {
            let nested = HarnessCtx {
                depth: self.ctx.depth + 1,
                ..self.ctx.clone_for_nested()
            };
            match self
                .await_bounded(tokio::time::timeout(
                    tool.timeout(),
                    Box::pin(crate::harness::execute(
                        &nested,
                        tool_name,
                        tool_args.clone(),
                    )),
                ))
                .await
            {
                Ok(Ok(Ok(outcome))) => {
                    self.events.extend(outcome.events);
                    crate::tool_envelope::normalize(outcome.value, tool.envelope_family())
                }
                Ok(Ok(Err(call_error))) => crate::tool_envelope::normalize_error(
                    &call_error,
                    crate::tool_envelope::Cause::default(),
                    tool.envelope_family(),
                ),
                Ok(Err(_)) => crate::tool_envelope::normalize_error(
                    "Plan call timed out. Its outcome may be unknown; do not automatically retry.",
                    crate::tool_envelope::Cause {
                        timeout: true,
                        ..Default::default()
                    },
                    tool.envelope_family(),
                ),
                Err(abort) => {
                    let result = interrupted(&abort);
                    if let Ok(store) = self.ctx.state.database() {
                        let _ = store.update_message(
                            &row.id,
                            &json!({"request": audit, "result": result}).to_string(),
                            "",
                            "complete",
                        );
                    }
                    self.block_signature(&signature);
                    return Err(abort);
                }
            }
        } else {
            let mcp_ui = tool.is_mcp().then(|| {
                crate::mcp_input::CallUi::new(
                    self.ctx.state.mcp_inputs.clone(),
                    self.ctx.emit.clone(),
                )
            });
            match self
                .await_bounded(tokio::time::timeout(
                    tool.timeout(),
                    tool.call_with_input(tool_args.clone(), None, mcp_ui),
                ))
                .await
            {
                Ok(Ok(Ok(value))) => crate::tool_envelope::normalize(value, tool.envelope_family()),
                Ok(Ok(Err(call_error))) => crate::tool_envelope::normalize_error(
                    &call_error,
                    crate::tool_envelope::Cause::default(),
                    tool.envelope_family(),
                ),
                Ok(Err(_)) => crate::tool_envelope::normalize_error(
                    "Plan call timed out. Its outcome may be unknown; do not automatically retry.",
                    crate::tool_envelope::Cause {
                        timeout: true,
                        ..Default::default()
                    },
                    tool.envelope_family(),
                ),
                Err(abort) => {
                    let result = interrupted(&abort);
                    if let Ok(store) = self.ctx.state.database() {
                        let _ = store.update_message(
                            &row.id,
                            &json!({"request": audit, "result": result}).to_string(),
                            "",
                            "complete",
                        );
                    }
                    self.block_signature(&signature);
                    return Err(abort);
                }
            }
        };
        let (bounded, maybe_artifact) = crate::artifacts::bound_tool_result(
            outcome,
            &tool.tool.name,
            &self.ctx.conversation_id,
            Some(&self.ctx.run_id),
            crate::artifacts::DEFAULT_MAX_RESULT_CHARS,
        );
        if let Some(artifact) = &maybe_artifact {
            if let Ok(store) = self.ctx.state.database() {
                let _ = store.save_artifact(artifact);
            }
        }
        {
            let store = self.ctx.state.database().map_err(error)?;
            store
                .update_message(
                    &row.id,
                    &json!({"request": audit, "result": bounded}).to_string(),
                    "",
                    "complete",
                )
                .map_err(error)?;
        }
        let failed = crate::tool_envelope::failed(&bounded);
        if failed {
            // An error or timed-out call cannot be proven unchanged — block
            // the signature for the rest of the turn, exactly like the
            // parent's non-retryable Stop.
            self.block_signature(&signature);
        }
        self.calls.push(json!({"tool": tool_name, "status": if failed { "error" } else { "ok" }, "result": bounded, "callId": call_id}));
        Ok((bounded, failed))
    }

    async fn run_call(&mut self, spec: &Value) -> Step<()> {
        let tool_name = spec
            .get("tool")
            .and_then(Value::as_str)
            .ok_or_else(|| error("'call' needs a 'tool' alias."))?
            .to_string();
        let args_expr = spec.get("args").cloned().unwrap_or_else(|| json!({}));
        let args = self.eval(&args_expr, 0)?;
        if !args.is_object() || args.to_string().len() > 32_768 {
            return Err(error(format!(
                "'call' args for '{tool_name}' must be a JSON object under 32 KiB."
            )));
        }
        self.tool_calls += 1;
        if self.tool_calls > MAX_CALLS {
            return Err(error(format!("Plan exceeds {MAX_CALLS} tool calls.")));
        }
        let call_id = format!("plan-{}-{}", self.nonce, self.tool_calls);
        let (result, failed) = self
            .dispatch_call(&tool_name, args, &call_id, false)
            .await?;
        if let Some(name) = spec.get("as").and_then(Value::as_str) {
            self.bind(name, result.clone())?;
        }
        if failed {
            return Err(error(format!(
                "Tool '{tool_name}' failed; the plan aborted. Its outcome is in 'calls'."
            )));
        }
        Ok(())
    }

    /// Explicit independent-read fan-out. Every member must be a `call`
    /// whose tool is a classified trusted read, auto-allowed under the current
    /// access mode, and whose args reference only outer-scope variables.
    /// Member results bind in statement order after the join — audit rows and
    /// per-call normalization are identical to sequential calls.
    async fn run_parallel(&mut self, body: &[Value]) -> Step<()> {
        if body.is_empty() || body.len() > 4 {
            return Err(error("'parallel' takes 1-4 call statements."));
        }
        struct Member {
            tool_name: String,
            args: Value,
            bind: Option<String>,
            call_id: String,
        }
        let mut members = Vec::new();
        for statement in body.iter() {
            let call = statement
                .get("call")
                .ok_or_else(|| error("'parallel' members must be 'call' statements."))?;
            let tool_name = call
                .get("tool")
                .and_then(Value::as_str)
                .ok_or_else(|| error("'parallel' member needs a 'tool' alias."))?
                .to_string();
            let args_expr = call.get("args").cloned().unwrap_or_else(|| json!({}));
            let args = self.eval(&args_expr, 0)?;
            if !args.is_object() || args.to_string().len() > 32_768 {
                return Err(error(format!(
                    "'parallel' args for '{tool_name}' must be a JSON object under 32 KiB."
                )));
            }
            self.tool_calls += 1;
            if self.tool_calls > MAX_CALLS {
                return Err(error(format!("Plan exceeds {MAX_CALLS} tool calls.")));
            }
            let tool = self
                .ctx
                .inherit_tools
                .iter()
                .find(|tool| tool.alias == tool_name)
                .ok_or_else(|| {
                    error(format!("Tool '{tool_name}' is not offered in this preset."))
                })?;
            if !tool.concurrent_read() {
                return Err(error(format!(
                    "Tool '{tool_name}' is not a trusted read and cannot join a 'parallel' block."
                )));
            }
            let bind = call.get("as").and_then(Value::as_str).map(str::to_string);
            let call_id = format!("plan-{}-{}", self.nonce, self.tool_calls);
            members.push(Member {
                tool_name,
                args,
                bind,
                call_id,
            });
        }
        // `let` writes from sibling members can never feed each other: bindings
        // land only after every member settles, in statement order.
        let ctx = self.ctx;
        let futures: Vec<_> = members
            .iter()
            .map(|member| {
                let tool = ctx.inherit_tools.iter().find(|tool| tool.alias == member.tool_name).expect("checked above");
                let tool_name = member.tool_name.clone();
                let args = member.args.clone();
                let mut cancel = self.cancel.clone();
                async move {
                    // A cancel the executor already recorded never re-fires
                    // `changed()` — check the stored flag before dispatching.
                    if *cancel.borrow() {
                        return Err(MemberError::Cancelled);
                    }
                    tokio::select! {
                        _ = cancel.changed() => Err(MemberError::Cancelled),
                        result = async {
                            let tool_args = tool.prepare_arguments(args).map_err(MemberError::Failed)?;
                            // Capability kills deny at dispatch too — the same
                            // check the sequential path applies. Without it a
                            // `parallel` member that is a capability-gated
                            // native read would run while the kill is saved.
                            if let Some(reason) = crate::capabilities::tool_capability(tool).and_then(|capability| {
                                crate::capabilities::ToolPolicy::guard(ctx.state.database()).denial(capability)
                            }) {
                                return Err(MemberError::Failed(reason));
                            }
                            crate::continuation::admit_tool(ctx.state,&ctx.run_id,&tool_name,&tool_args).map_err(MemberError::Failed)?;
                            // Turn-scoped replay protection, identical to the
                            // sequential gate — checked before any dispatch.
                            let signature = crate::continuation::tool_signature(tool, &tool_args);
                            let blocked=ctx.state.turn_blocked.lock().map_err(|_|MemberError::Failed("Turn replay-protection registry is unavailable.".into()))?.get(&ctx.run_id).cloned();
                            let already_blocked=match blocked{Some(set)=>set.lock().map_err(|_|MemberError::Failed("Turn replay-protection state is unavailable.".into()))?.contains(&signature),None=>false};
                            if already_blocked {
                                return Err(MemberError::Failed("Not repeated: an earlier attempt this turn had an uncertain outcome.".into()));
                            }
                            let automatic = !tool.requires_approval(&tool_args)
                                && ctx.access_mode.automatic_reason(tool.trusted_read()).is_some();
                            if !automatic {
                                return Err(MemberError::Failed(format!("Tool '{tool_name}' needs approval and cannot join a 'parallel' block.")));
                            }
                            if tool.is_harness() {
                                let nested = HarnessCtx { depth: ctx.depth + 1, ..ctx.clone_for_nested() };
                                tokio::time::timeout(tool.timeout(), crate::harness::execute(&nested, &tool_name, tool_args))
                                    .await
                                    .map_err(|_| MemberError::TimedOut)?
                                    .map(|outcome| (outcome.value, outcome.events))
                                    .map_err(MemberError::Failed)
                            } else {
                                let mcp_ui = tool.is_mcp().then(|| {
                                    crate::mcp_input::CallUi::new(ctx.state.mcp_inputs.clone(), ctx.emit.clone())
                                });
                                tokio::time::timeout(tool.timeout(), tool.call_with_input(tool_args, None, mcp_ui))
                                    .await
                                    .map_err(|_| MemberError::TimedOut)?
                                    .map(|value| (value, Vec::new()))
                                    .map_err(MemberError::Failed)
                            }
                        } => result,
                    }
                }
            })
            .collect();
        let settled = futures_util::future::join_all(futures).await;
        // Every member that dispatched gets its call record and durable audit
        // row in statement order — a failure in one must never hide the
        // outcome of another that already ran. The plan aborts only after the
        // whole group is accounted for.
        let mut abort: Option<Abort> = None;
        for (member, result) in members.iter().zip(settled) {
            let call_id = member.call_id.clone();
            let tool = self
                .ctx
                .inherit_tools
                .iter()
                .find(|tool| tool.alias == member.tool_name)
                .expect("checked above");
            let outcome = match result {
                Ok((value, events)) => {
                    self.events.extend(events);
                    crate::tool_envelope::normalize(value, tool.envelope_family())
                }
                Err(MemberError::Cancelled) => crate::tool_envelope::normalize_error(
                    "Plan cancelled.",
                    crate::tool_envelope::Cause {
                        cancelled: true,
                        ..Default::default()
                    },
                    tool.envelope_family(),
                ),
                Err(MemberError::TimedOut) => crate::tool_envelope::normalize_error(
                    "Plan call timed out. Its outcome may be unknown; do not automatically retry.",
                    crate::tool_envelope::Cause {
                        timeout: true,
                        ..Default::default()
                    },
                    tool.envelope_family(),
                ),
                Err(MemberError::Failed(message)) => crate::tool_envelope::normalize_error(
                    &message,
                    crate::tool_envelope::Cause::default(),
                    tool.envelope_family(),
                ),
            };
            let (bounded, maybe_artifact) = crate::artifacts::bound_tool_result(
                outcome,
                &tool.tool.name,
                &self.ctx.conversation_id,
                Some(&self.ctx.run_id),
                crate::artifacts::DEFAULT_MAX_RESULT_CHARS,
            );
            if let Some(artifact) = &maybe_artifact {
                if let Ok(store) = self.ctx.state.database() {
                    let _ = store.save_artifact(artifact);
                }
            }
            let audit = json!({"connector": tool.connector, "name": tool.tool.name, "arguments": member.args,
                "decision": "allowed", "accessMode": self.ctx.access_mode,
                "authorization": "auto-approved plan parallel read",
                "program": true, "callId": call_id, "stepId": self.ctx.step_id});
            {
                let store = self.ctx.state.database().map_err(error)?;
                store
                    .append_message(
                        &self.ctx.conversation_id,
                        "tool",
                        &json!({"request": audit, "result": bounded}).to_string(),
                        "complete",
                    )
                    .map_err(error)?;
            }
            let failed = crate::tool_envelope::failed(&bounded);
            if failed {
                // Same non-retryable rule as sequential calls: an uncertain
                // member outcome blocks its signature for the rest of the turn.
                if let Ok(prepared) = tool.prepare_arguments(member.args.clone()) {
                    self.block_signature(&crate::continuation::tool_signature(tool, &prepared));
                }
            }
            let cancelled =
                bounded.pointer("/error/code").and_then(Value::as_str) == Some("cancelled");
            self.calls.push(json!({"tool": member.tool_name, "status": if cancelled { "cancelled" } else if failed { "error" } else { "ok" }, "result": bounded, "callId": call_id}));
            if let Some(name) = &member.bind {
                self.bind(name, bounded.clone())?;
            }
            if abort.is_none() && (failed || cancelled) {
                abort = Some(if cancelled {
                    Abort {
                        status: "cancelled",
                        message: "Plan cancelled.".into(),
                    }
                } else {
                    error(format!(
                        "Tool '{}' failed; the plan aborted.",
                        member.tool_name
                    ))
                });
            }
        }
        if let Some(abort) = abort {
            return Err(abort);
        }
        Ok(())
    }
}

fn kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Array(_) => "an array",
        Value::Object(_) => "an object",
    }
}

fn truthy(value: &Value) -> bool {
    match value {
        Value::Bool(flag) => *flag,
        Value::Null => false,
        _ => true,
    }
}

/// Deterministic scalar ordering: numbers by value, strings lexically,
/// bools/null equal-kind only. Mixed kinds are not comparable — no implicit
/// coercion can hide a dataflow bug.
fn compare(a: &Value, b: &Value) -> Option<std::cmp::Ordering> {
    use std::cmp::Ordering;
    match (a, b) {
        (Value::Null, Value::Null) => Some(Ordering::Equal),
        (Value::Bool(x), Value::Bool(y)) => Some(x.cmp(y)),
        (Value::Number(x), Value::Number(y)) => {
            // Exact integer ordering first — f64 cannot distinguish values
            // at/above 2^53 and would collapse them to Equal.
            if let (Some(a), Some(b)) = (x.as_i64(), y.as_i64()) {
                Some(a.cmp(&b))
            } else if let (Some(a), Some(b)) = (x.as_u64(), y.as_u64()) {
                Some(a.cmp(&b))
            } else if let (Some(a), Some(b)) = (x.as_i64(), y.as_u64()) {
                Some(if a < 0 {
                    Ordering::Less
                } else {
                    (a as u64).cmp(&b)
                })
            } else if let (Some(a), Some(b)) = (x.as_u64(), y.as_i64()) {
                Some(if b < 0 {
                    Ordering::Greater
                } else {
                    a.cmp(&(b as u64))
                })
            } else {
                x.as_f64()?.partial_cmp(&y.as_f64()?)
            }
        }
        (Value::String(x), Value::String(y)) => Some(x.cmp(y)),
        (Value::Array(x), Value::Array(y)) => {
            if x.len() != y.len() {
                return Some(x.len().cmp(&y.len()));
            }
            for (left, right) in x.iter().zip(y.iter()) {
                let ordering = compare(left, right)?;
                if ordering != Ordering::Equal {
                    return Some(ordering);
                }
            }
            Some(Ordering::Equal)
        }
        (Value::Object(x), Value::Object(y)) => {
            if x == y {
                Some(Ordering::Equal)
            } else {
                Some(
                    x.len()
                        .cmp(&y.len())
                        .then_with(|| format!("{x:?}").cmp(&format!("{y:?}"))),
                )
            }
        }
        _ => None,
    }
}

/// `ptc_run` executable plan arm. Returns `None` when the call used the
/// legacy `steps` shape instead.
pub async fn execute_plan(ctx: &HarnessCtx<'_>, plan: &Value) -> Result<HarnessOutcome, String> {
    let Value::Object(root) = plan else {
        return Err(
            "ptc_run 'plan' must be a JSON object: {\"body\": [...], \"return\": <expr>}.".into(),
        );
    };
    for key in FORBIDDEN_ROOT_KEYS {
        if root.contains_key(key) {
            return Err(format!("ptc_run plan cannot carry a '{key}' field — permissions belong to the host dispatcher."));
        }
    }
    let body = root
        .get("body")
        .and_then(Value::as_array)
        .ok_or("ptc_run 'plan' needs a 'body' statement array.")?;
    let mut nodes = 0;
    validate_node(plan, 0, &mut nodes).map_err(|abort| abort.message)?;
    let mut exec = Exec::new(ctx);
    let outcome = exec.run_statements(body, 0, true).await;
    // A `return`-eval failure is an abort like any other — the completed call
    // evidence still reaches the caller, never a bare Err that drops `calls`.
    let (plan_status, message, value) = match outcome {
        Ok(()) => match root.get("return") {
            Some(expr) => match exec.eval(expr, 0) {
                Ok(value) => ("ok", None, value),
                Err(abort) => (abort.status, Some(abort.message), Value::Null),
            },
            None => {
                let mut snapshot = Map::new();
                for scope in &exec.scopes {
                    for (name, value) in scope {
                        snapshot.insert(name.clone(), value.clone());
                    }
                }
                ("ok", None, Value::Object(snapshot))
            }
        },
        Err(abort) => (abort.status, Some(abort.message), Value::Null),
    };
    // Auditable completion record: `calls` in dispatch order, `partial` the
    // calls that completed, `unresolved` the top-level statement indices that
    // never started. The envelope is the model-visible contract; the record
    // fields are also exposed at the top level like every other compat copy.
    let record = json!({
        "calls": exec.calls,
        "value": value,
        "partial": exec.calls.iter().filter(|call| call["status"] == "ok").count(),
        "unresolved": (exec.top_attempted..body.len()).collect::<Vec<_>>(),
    });
    // An aborted plan is never automatically replayable: a completed write
    // inside it may have applied real side effects, so failure certainty is
    // conservative (`unknown`) except a denial, which never dispatched.
    let (status, error) = match plan_status {
        "ok" => (crate::tool_envelope::Status::Success, None),
        "denied" => (
            crate::tool_envelope::Status::Failure,
            Some(crate::tool_envelope::error_object(
                "denied",
                &message.unwrap_or_default(),
                false,
                crate::tool_envelope::SIDE_NONE,
            )),
        ),
        "cancelled" => (
            crate::tool_envelope::Status::Cancelled,
            Some(crate::tool_envelope::error_object(
                "cancelled",
                &message.unwrap_or_default(),
                false,
                crate::tool_envelope::SIDE_UNKNOWN,
            )),
        ),
        _ => (
            crate::tool_envelope::Status::Failure,
            Some(crate::tool_envelope::error_object(
                "plan-failed",
                &message.unwrap_or_default(),
                false,
                crate::tool_envelope::SIDE_UNKNOWN,
            )),
        ),
    };
    let mut result = crate::tool_envelope::make(
        status,
        record.clone(),
        Vec::new(),
        error,
        json!({"family": crate::tool_envelope::Family::Harness.as_str(), "truncated": false}),
    );
    crate::tool_envelope::merge_compat(&mut result, &record);
    let mut outcome = HarnessOutcome::value(result);
    // Nested harness events (audit/subagent/control surfaced inside calls)
    // propagate ahead of the plan summary — nothing dispatched is invisible.
    outcome.events.extend(exec.events);
    Ok(outcome.event(
        "ptc",
        json!({"plan": true, "calls": exec.calls.len(), "status": status.as_str()}),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::Arc;

    /// Real conversation + run rows so audit message writes satisfy FKs.
    fn fixture<'a>(
        state: &'a crate::AppState,
        workspace: &std::path::Path,
        _conversation: &str,
        tools: &'a [crate::connectors::AgentTool],
        access_mode: crate::permissions::AccessMode,
        emit: Arc<dyn Fn(String, Option<Value>) + Send + Sync>,
    ) -> HarnessCtx<'a> {
        let aliases: Vec<&str> = tools.iter().map(|tool| tool.alias.as_str()).collect();
        let (conversation_id, run_id) = {
            let store = state.database().unwrap();
            let conversation = store.create_conversation().unwrap();
            let now = crate::store::now();
            let run = crate::agent_run::RunRecord {
                id: uuid::Uuid::new_v4().to_string(),
                conversation_id: conversation.id.clone(),
                status: crate::agent_run::RunState::ExecutingTools,
                model_provider: None,
                model_id: Some("fixture".into()),
                checkpoint: None,
                error: None,
                created_at: now,
                updated_at: now,
            };
            store.save_run(&run).unwrap();
            (conversation.id, run.id)
        };
        HarnessCtx {
            state,
            snapshot: crate::test_support::snapshot(state, workspace, &aliases),
            conversation_id,
            run_id,
            step_id: "plan-step".into(),
            access_mode,
            depth: 0,
            preset_id: crate::presets::CODE.into(),
            inherit_tools: tools,
            emit,
        }
    }

    fn workspace_tools(
        state: &crate::AppState,
        workspace: &std::path::Path,
    ) -> Vec<crate::connectors::AgentTool> {
        let _ = state;
        crate::workspace::Workspace::open(workspace.to_str().unwrap())
            .map(|workspace| Arc::new(workspace).tools())
            .unwrap()
    }

    async fn run_plan(ctx: &HarnessCtx<'_>, plan: Value) -> Value {
        execute_plan(ctx, &plan)
            .await
            .map(|outcome| outcome.value)
            .unwrap_or_else(|error| json!({"status": "rejected", "error": error}))
    }

    #[tokio::test]
    async fn plan_binds_evaluates_and_returns_values() {
        let dir = tempfile::tempdir().unwrap();
        let state = crate::test_support::app_state(dir.path());
        let tools = workspace_tools(&state, dir.path());
        let ctx = fixture(
            &state,
            dir.path(),
            "c1",
            &tools,
            crate::permissions::AccessMode::FullAccess,
            Arc::new(|_, _| {}),
        );
        let result = run_plan(&ctx, json!({
            "body": [
                {"let": {"name": "items", "value": [1, 2, 3, 4]}},
                {"let": {"name": "doubled", "value": {"op": "map", "list": {"$": "items"}, "as": "n", "select": {"op": "eq", "a": {"$": "n"}, "b": 2}}}},
                {"let": {"name": "count", "value": {"op": "len", "of": {"$": "doubled"}}}},
            ],
            "return": {"op": "merge", "args": [{"count": {"$": "count"}}, {"picked": {"$": "doubled"}}]}
        }))
        .await;
        assert_eq!(result["status"], "success", "{result}");
        assert_eq!(result["value"]["count"], 4);
        assert_eq!(
            result["value"]["picked"],
            json!([false, true, false, false])
        );
        assert_eq!(result["calls"], json!([]));
    }

    #[tokio::test]
    async fn plan_result_of_one_call_feeds_the_next_calls_arguments() {
        // The real dataflow contract: a workspace read yields the sha256 an
        // edit requires — the model could not know it before dispatch.
        let dir = tempfile::tempdir().unwrap();
        let state = crate::test_support::app_state(dir.path());
        let workspace = dir.path().join("ws");
        std::fs::create_dir_all(&workspace).unwrap();
        std::fs::write(workspace.join("app.txt"), "let answer = 41;\n").unwrap();
        let tools = workspace_tools(&state, &workspace);
        let ctx = fixture(
            &state,
            &workspace,
            "c2",
            &tools,
            crate::permissions::AccessMode::FullAccess,
            Arc::new(|_, _| {}),
        );
        let result = run_plan(&ctx, json!({
            "body": [
                {"call": {"tool": "workspace_read_file", "args": {"path": "app.txt"}, "as": "before"}},
                {"call": {"tool": "workspace_edit_file", "args": {
                    "path": "app.txt",
                    "expected_sha256": {"$": "before", "path": ["data", "sha256"]},
                    "old_text": "41",
                    "new_text": "42",
                }, "as": "edited"}},
            ],
            "return": {"op": "get", "of": {"$": "edited"}, "path": ["status"]}
        }))
        .await;
        assert_eq!(result["status"], "success", "{result}");
        assert_eq!(result["calls"].as_array().unwrap().len(), 2);
        assert_eq!(result["calls"][0]["tool"], "workspace_read_file");
        assert_eq!(result["calls"][1]["tool"], "workspace_edit_file");
        assert!(
            result["calls"]
                .as_array()
                .unwrap()
                .iter()
                .all(|entry| entry["status"] == "ok"),
            "{result}"
        );
        assert_eq!(
            std::fs::read_to_string(workspace.join("app.txt")).unwrap(),
            "let answer = 42;\n"
        );
    }

    #[tokio::test]
    async fn plan_branches_on_call_results_both_ways() {
        let dir = tempfile::tempdir().unwrap();
        let state = crate::test_support::app_state(dir.path());
        let workspace = dir.path().join("ws");
        std::fs::create_dir_all(&workspace).unwrap();
        std::fs::write(workspace.join("data.txt"), "42\n").unwrap();
        let tools = workspace_tools(&state, &workspace);
        for (needle, expected) in [("42", "found"), ("absent", "missing")] {
            let ctx = fixture(
                &state,
                &workspace,
                "c3",
                &tools,
                crate::permissions::AccessMode::FullAccess,
                Arc::new(|_, _| {}),
            );
            let result = run_plan(&ctx, json!({
                "body": [
                    {"call": {"tool": "workspace_read_file", "args": {"path": "data.txt"}, "as": "page"}},
                    {"if": {
                        "cond": {"op": "contains", "of": {"$": "page", "path": ["data", "content"]}, "item": needle},
                        "then": [{"let": {"name": "verdict", "value": "found"}}],
                        "else": [{"let": {"name": "verdict", "value": "missing"}}],
                    }},
                ],
                "return": {"$": "verdict"}
            }))
            .await;
            assert_eq!(result["status"], "success", "{result}");
            assert_eq!(result["value"], expected, "{needle}");
        }
    }

    #[tokio::test]
    async fn plan_bounded_loop_and_transform_consume_results() {
        // Temporary-workspace example: enumerate a directory, read each file,
        // accumulate the contents, transform to a report.
        let dir = tempfile::tempdir().unwrap();
        let state = crate::test_support::app_state(dir.path());
        let workspace = dir.path().join("ws");
        std::fs::create_dir_all(workspace.join("src")).unwrap();
        std::fs::write(workspace.join("src/a.txt"), "alpha").unwrap();
        std::fs::write(workspace.join("src/b.txt"), "beta").unwrap();
        let tools = workspace_tools(&state, &workspace);
        let ctx = fixture(
            &state,
            &workspace,
            "c4",
            &tools,
            crate::permissions::AccessMode::FullAccess,
            Arc::new(|_, _| {}),
        );
        let result = run_plan(&ctx, json!({
            "body": [
                {"call": {"tool": "workspace_list_files", "args": {"path": "src"}, "as": "listing"}},
                {"let": {"name": "names", "value": {"op": "map", "list": {"$": "listing", "path": ["data", "entries"]}, "as": "e", "select": {"op": "get", "of": {"$": "e"}, "path": ["name"]}}}},
                {"let": {"name": "chunks", "value": []}},
                {"for": {"var": "name", "in": {"$": "names"}, "body": [
                    {"call": {"tool": "workspace_read_file", "args": {"path": {"op": "concat", "args": ["src/", {"$": "name"}]}}, "as": "page"}},
                    {"let": {"name": "chunks", "value": {"op": "concat", "args": [{"$": "chunks"}, [{"$": "page", "path": ["data", "content"]}]]}}},
                ]}},
            ],
            "return": {"op": "join", "list": {"$": "chunks"}, "sep": "|"}
        }))
        .await;
        assert_eq!(result["status"], "success", "{result}");
        let value = result["value"].as_str().unwrap();
        assert!(value.contains("alpha") && value.contains("beta"), "{value}");
        assert_eq!(
            result["calls"].as_array().unwrap().len(),
            3,
            "list + two reads"
        );
    }

    #[tokio::test]
    async fn plan_parallel_block_runs_independent_reads_in_order() {
        let dir = tempfile::tempdir().unwrap();
        let state = crate::test_support::app_state(dir.path());
        let workspace = dir.path().join("ws");
        std::fs::create_dir_all(&workspace).unwrap();
        std::fs::write(workspace.join("a.txt"), "alpha").unwrap();
        std::fs::write(workspace.join("b.txt"), "beta").unwrap();
        let tools = workspace_tools(&state, &workspace);
        let ctx = fixture(
            &state,
            &workspace,
            "c5",
            &tools,
            crate::permissions::AccessMode::AutoApprove,
            Arc::new(|_, _| {}),
        );
        let result = run_plan(&ctx, json!({
            "body": [
                {"parallel": {"body": [
                    {"call": {"tool": "workspace_read_file", "args": {"path": "a.txt"}, "as": "first"}},
                    {"call": {"tool": "workspace_read_file", "args": {"path": "b.txt"}, "as": "second"}},
                ]}},
            ],
            "return": [{"$": "first", "path": ["data", "content"]}, {"$": "second", "path": ["data", "content"]}]
        }))
        .await;
        assert_eq!(result["status"], "success", "{result}");
        // Bindings land in statement order even if completion was unordered.
        let pair = result["value"].as_array().unwrap();
        assert!(pair[0].as_str().unwrap().contains("alpha"), "{pair:?}");
        assert!(pair[1].as_str().unwrap().contains("beta"), "{pair:?}");
    }

    #[tokio::test]
    async fn plan_rejects_mutations_and_approval_gated_calls_in_parallel() {
        let dir = tempfile::tempdir().unwrap();
        let state = crate::test_support::app_state(dir.path());
        let workspace = dir.path().join("ws");
        std::fs::create_dir_all(&workspace).unwrap();
        let tools = workspace_tools(&state, &workspace);
        let ctx = fixture(
            &state,
            &workspace,
            "c6",
            &tools,
            crate::permissions::AccessMode::FullAccess,
            Arc::new(|_, _| {}),
        );
        let result = run_plan(&ctx, json!({
            "body": [{"parallel": {"body": [
                {"call": {"tool": "workspace_create_file", "args": {"path": "x.txt", "content": "x"}}},
                {"call": {"tool": "workspace_list_files", "args": {"path": "."}}},
            ]}}],
        }))
        .await;
        assert_ne!(result["status"], "success", "{result}");
        assert!(
            !workspace.join("x.txt").exists(),
            "rejected parallel member still ran"
        );
        // Under Ask, even trusted reads need approval — parallel refuses them
        // rather than opening parallel dialogs.
        let ctx = fixture(
            &state,
            &workspace,
            "c7",
            &tools,
            crate::permissions::AccessMode::Ask,
            Arc::new(|_, _| {}),
        );
        let result = run_plan(
            &ctx,
            json!({
                "body": [{"parallel": {"body": [
                    {"call": {"tool": "workspace_list_files", "args": {"path": "."}}},
                    {"call": {"tool": "workspace_list_files", "args": {"path": "."}}},
                ]}}],
            }),
        )
        .await;
        assert_ne!(result["status"], "success", "{result}");
    }

    #[tokio::test]
    async fn plan_enforces_alias_schema_and_shape_gates() {
        let dir = tempfile::tempdir().unwrap();
        let state = crate::test_support::app_state(dir.path());
        let workspace = dir.path().join("ws");
        std::fs::create_dir_all(&workspace).unwrap();
        let tools = workspace_tools(&state, &workspace);
        let ctx = fixture(
            &state,
            &workspace,
            "c8",
            &tools,
            crate::permissions::AccessMode::FullAccess,
            Arc::new(|_, _| {}),
        );
        for (name, plan) in [
            (
                "unknown alias",
                json!({"body": [{"call": {"tool": "nope", "args": {}}}]}),
            ),
            (
                "missing args key",
                json!({"body": [{"call": {"tool": "workspace_read_file", "args": {"bogus": 1}}}]}),
            ),
            (
                "non-object args",
                json!({"body": [{"call": {"tool": "workspace_read_file", "args": ["a"]}}]}),
            ),
            (
                "nested ptc_run",
                json!({"body": [{"call": {"tool": "ptc_run", "args": {"plan": {"body": []}}}}]}),
            ),
            (
                "finish",
                json!({"body": [{"call": {"tool": "finish", "args": {"answer": "x"}}}]}),
            ),
            (
                "missing variable",
                json!({"body": [{"let": {"name": "v", "value": {"$": "ghost"}}}]}),
            ),
            (
                "missing path",
                json!({"body": [{"let": {"name": "v", "value": {"$": "ghost", "path": ["x"]}}}]}),
            ),
            (
                "forbidden key",
                json!({"body": [{"let": {"name": "v", "value": {"__proto__": 1}}}]}),
            ),
            (
                "model-written authority",
                json!({"allow": true, "body": []}),
            ),
            (
                "if needs bool",
                json!({"body": [{"if": {"cond": "yes", "then": []}}]}),
            ),
            (
                "for needs array",
                json!({"body": [{"for": {"var": "x", "in": 5, "body": []}}]}),
            ),
        ] {
            let result = run_plan(&ctx, plan).await;
            assert_ne!(result["status"], "success", "{name}: {result}");
        }
        // The ptc_run schema itself enforces exactly-one-of plan|steps.
        let tool = tools
            .iter()
            .find(|tool| tool.alias == "ptc_run")
            .cloned()
            .unwrap_or_else(|| crate::connectors::AgentTool::harness("ptc_run").unwrap());
        assert!(tool
            .prepare_arguments(json!({"plan": {"body": []}}))
            .is_ok());
        assert!(tool
            .prepare_arguments(json!({"steps": [{"tool": "a"}]}))
            .is_ok());
        assert!(tool.prepare_arguments(json!({})).is_err());
        assert!(tool
            .prepare_arguments(json!({"plan": {"body": []}, "extra": 1}))
            .is_err());
    }

    #[tokio::test]
    async fn plan_denial_capability_kill_and_cancel_are_audited() {
        let dir = tempfile::tempdir().unwrap();
        // Arc'd so the emit-side approval resolver can own a handle to the
        // same Approvals map the dispatcher asks through.
        let state = Arc::new(crate::test_support::app_state(dir.path()));
        let workspace = dir.path().join("ws");
        std::fs::create_dir_all(&workspace).unwrap();
        std::fs::write(workspace.join("a.txt"), "alpha").unwrap();
        let tools = workspace_tools(&state, &workspace);
        // User denial under Ask: the emit-side resolver answers the approval
        // prompt synchronously, the plan aborts and the call row is audited.
        let resolver = state.clone();
        let emit: Arc<dyn Fn(String, Option<Value>) + Send + Sync> =
            Arc::new(move |_activity, approval| {
                if let Some(id) = approval.and_then(|payload| {
                    payload
                        .get("id")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                }) {
                    let _ = resolver.approvals.resolve(&id, false);
                }
            });
        let conversation = "c9";
        let ctx = fixture(
            &state,
            &workspace,
            conversation,
            &tools,
            crate::permissions::AccessMode::Ask,
            emit,
        );
        let result = run_plan(&ctx, json!({
            "body": [
                {"call": {"tool": "workspace_list_files", "args": {"path": "."}, "as": "never"}},
                {"call": {"tool": "workspace_list_files", "args": {"path": "."}}},
            ],
        }))
        .await;
        assert_eq!(result["status"], "failure", "{result}");
        assert_eq!(result["error"]["code"], "denied", "{result}");
        assert_eq!(result["unresolved"], json!([1]), "{result}");
        // Capability kill denies at plan dispatch exactly like direct calls.
        state
            .database()
            .unwrap()
            .save_setting(
                crate::capabilities::DISABLED_KEY,
                &vec!["workspace_files".to_string()],
            )
            .unwrap();
        let ctx = fixture(
            &state,
            &workspace,
            conversation,
            &tools,
            crate::permissions::AccessMode::FullAccess,
            Arc::new(|_, _| {}),
        );
        let result = run_plan(
            &ctx,
            json!({"body": [{"call": {"tool": "workspace_list_files", "args": {"path": "."}}}]}),
        )
        .await;
        assert_eq!(result["status"], "failure", "{result}");
        assert_eq!(result["error"]["code"], "denied", "{result}");
        state
            .database()
            .unwrap()
            .save_setting(crate::capabilities::DISABLED_KEY, &Vec::<String>::new())
            .unwrap();
        // Cancellation aborts at the next statement boundary.
        state.cancel.send_replace(true);
        let ctx = fixture(
            &state,
            &workspace,
            conversation,
            &tools,
            crate::permissions::AccessMode::FullAccess,
            Arc::new(|_, _| {}),
        );
        let result = run_plan(
            &ctx,
            json!({"body": [{"call": {"tool": "workspace_list_files", "args": {"path": "."}}}]}),
        )
        .await;
        assert_eq!(result["status"], "cancelled", "{result}");
        state.cancel.send_replace(false);
    }

    /// A nested call must charge the *shared* turn budget before dispatch —
    /// an exhausted family counter denies the call without running it, and
    /// the already-open audit row settles instead of staying streaming.
    #[tokio::test]
    async fn plan_nested_calls_charge_the_shared_turn_budget_before_dispatch() {
        let dir = tempfile::tempdir().unwrap();
        let state = crate::test_support::app_state(dir.path());
        let workspace = dir.path().join("ws");
        std::fs::create_dir_all(&workspace).unwrap();
        let tools = vec![crate::connectors::AgentTool::harness("web_search").unwrap()];
        let ctx = fixture(
            &state,
            &workspace,
            "c-budget",
            &tools,
            crate::permissions::AccessMode::FullAccess,
            Arc::new(|_, _| {}),
        );
        // The parent turn's search budget is already spent.
        {
            let mut map = state.turn_budgets.lock().unwrap();
            map.insert(
                ctx.run_id.clone(),
                std::sync::Arc::new(std::sync::Mutex::new(crate::continuation::Budgets {
                    searches_used: crate::continuation::Budgets::default().max_searches,
                    ..Default::default()
                })),
            );
        }
        let result = run_plan(
            &ctx,
            json!({"body": [{"call": {"tool": "web_search", "args": {"question": "locallm"}}}]}),
        )
        .await;
        assert_eq!(result["status"], "failure", "{result}");
        assert_eq!(result["error"]["code"], "denied", "{result}");
        assert_eq!(result["calls"][0]["status"], "denied", "{result}");
        // The opened audit row settled rather than staying streaming.
        let rows = state
            .database()
            .unwrap()
            .messages(&ctx.conversation_id)
            .unwrap();
        assert!(!rows.is_empty());
        assert!(
            rows.iter().all(|row| row.status != "streaming"),
            "a budget-denied call left a streaming audit row"
        );
        // The exhausted counter did not move past the cap — nothing ran.
        let spent = state
            .turn_budgets
            .lock()
            .unwrap()
            .get(&ctx.run_id)
            .unwrap()
            .lock()
            .unwrap()
            .searches_used;
        assert_eq!(spent, crate::continuation::Budgets::default().max_searches);
    }

    #[tokio::test]
    async fn plan_budgets_cap_calls_iterations_nodes_vars_and_time() {
        let dir = tempfile::tempdir().unwrap();
        let state = crate::test_support::app_state(dir.path());
        let workspace = dir.path().join("ws");
        std::fs::create_dir_all(&workspace).unwrap();
        let tools = workspace_tools(&state, &workspace);
        let ctx = fixture(
            &state,
            &workspace,
            "c10",
            &tools,
            crate::permissions::AccessMode::FullAccess,
            Arc::new(|_, _| {}),
        );
        // Call cap: the ninth call aborts even though each would be valid.
        let calls: Vec<Value> = (0..9)
            .map(|_| json!({"call": {"tool": "workspace_list_files", "args": {"path": "."}}}))
            .collect();
        let result = run_plan(&ctx, json!({"body": calls})).await;
        assert_ne!(result["status"], "success", "{result}");
        assert!(
            result["error"]["message"]
                .as_str()
                .unwrap()
                .contains("8 tool calls"),
            "{result}"
        );
        // Iteration cap: 65-element loop exceeds the 64 budget.
        let items: Vec<Value> = (0..65).map(Value::from).collect();
        let result = run_plan(&ctx, json!({"body": [
            {"for": {"var": "x", "in": items, "body": [{"let": {"name": "y", "value": {"$": "x"}}}]}},
        ]}))
        .await;
        assert!(
            result["error"]["message"]
                .as_str()
                .unwrap()
                .contains("iteration cap"),
            "{result}"
        );
        // Node cap: a deeply repeated body crosses 256 nodes at validation.
        let fat: Vec<Value> = (0..60)
            .map(|i| json!({"let": {"name": format!("v{i}"), "value": [1, 2, 3]}}))
            .collect();
        let result = run_plan(&ctx, json!({"body": fat})).await;
        assert_eq!(result["status"], "rejected", "{result}");
        assert!(
            result["error"].as_str().unwrap().contains("256"),
            "{result}"
        );
        // Variable bytes: a 70 KiB literal trips the 64 KiB variable budget.
        let result = run_plan(
            &ctx,
            json!({"body": [{"let": {"name": "big", "value": "x".repeat(70_000)}}]}),
        )
        .await;
        assert!(
            result["error"]["message"]
                .as_str()
                .unwrap()
                .contains("64 KiB"),
            "{result}"
        );
        // Time cap: force an expired deadline and the first guard fails.
        let mut exec = Exec::new(&ctx);
        exec.deadline = Instant::now() - std::time::Duration::from_secs(1);
        let abort = exec
            .run_statements(&[json!({"let": {"name": "v", "value": 1}})], 0, true)
            .await
            .unwrap_err();
        assert!(abort.message.contains("time budget"), "{abort:?}");
    }

    #[tokio::test]
    async fn plan_failed_call_reports_partial_without_replay() {
        let dir = tempfile::tempdir().unwrap();
        let state = crate::test_support::app_state(dir.path());
        let workspace = dir.path().join("ws");
        std::fs::create_dir_all(&workspace).unwrap();
        let tools = workspace_tools(&state, &workspace);
        let ctx = fixture(
            &state,
            &workspace,
            "c11",
            &tools,
            crate::permissions::AccessMode::FullAccess,
            Arc::new(|_, _| {}),
        );
        // edit_file on a missing file fails; the second call never runs and
        // the failure is surfaced, not retried.
        let result = run_plan(
            &ctx,
            json!({
                "body": [
                    {"call": {"tool": "workspace_list_files", "args": {"path": "."}, "as": "ok"}},
                    {"call": {"tool": "workspace_read_file", "args": {"path": "ghost.txt"}}},
                    {"call": {"tool": "workspace_list_files", "args": {"path": "."}}},
                ],
            }),
        )
        .await;
        assert_eq!(result["status"], "failure", "{result}");
        assert_eq!(result["partial"], 1, "{result}");
        assert_eq!(result["unresolved"], json!([2]), "{result}");
        assert_eq!(
            result["calls"].as_array().unwrap().len(),
            2,
            "no silent replay"
        );
        assert_eq!(result["calls"][1]["status"], "error", "{result}");
    }
}

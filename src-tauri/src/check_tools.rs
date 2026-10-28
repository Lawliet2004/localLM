//! Structured build/test results over the existing owned process lifecycle.
//! Parsers inspect bounded retained output; a successful exit alone never
//! becomes a claim that a test suite ran or passed.
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{collections::HashSet, sync::OnceLock};

const MAX_CASES: usize = 200;
const MAX_COUNT: u64 = 1_000_000;
const VERSION: &str = "locallm.check-report/1";

#[derive(Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum Format {
    #[default]
    Auto,
    Cargo,
    Vitest,
    Tap,
    Json,
}
impl Format {
    fn name(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Cargo => "cargo",
            Self::Vitest => "vitest",
            Self::Tap => "tap",
            Self::Json => "json",
        }
    }
}

#[derive(Default, Clone, Debug, Serialize, PartialEq, Eq)]
struct Counts {
    total: u64,
    passed: u64,
    failed: u64,
    skipped: u64,
    todo: u64,
}
impl Counts {
    fn checked(
        passed: u64,
        failed: u64,
        skipped: u64,
        todo: u64,
        total: Option<u64>,
    ) -> Result<Self, String> {
        let sum = passed
            .checked_add(failed)
            .and_then(|n| n.checked_add(skipped))
            .and_then(|n| n.checked_add(todo))
            .ok_or("Report test counts overflow.")?;
        if sum > MAX_COUNT || total.is_some_and(|n| n != sum) {
            return Err(
                "Report total must equal passed+failed+skipped+todo and be at most one million."
                    .into(),
            );
        }
        Ok(Self {
            total: sum,
            passed,
            failed,
            skipped,
            todo,
        })
    }
    fn add(&mut self, other: &Self) -> Result<(), String> {
        *self = Self::checked(
            self.passed + other.passed,
            self.failed + other.failed,
            self.skipped + other.skipped,
            self.todo + other.todo,
            None,
        )?;
        Ok(())
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Case {
    name: String,
    outcome: String,
    duration_ms: Option<f64>,
    source_index: usize,
    suite: Option<usize>,
    name_truncated: bool,
    name_sanitized: bool,
    diagnostic: Option<String>,
    diagnostic_truncated: bool,
}
fn case(name: &str, outcome: &str, duration_ms: Option<f64>) -> Case {
    let name_sanitized = name.chars().any(char::is_control);
    let length = name.chars().filter(|c| !c.is_control()).count();
    Case {
        name: name.chars().filter(|c| !c.is_control()).take(512).collect(),
        outcome: outcome.into(),
        duration_ms,
        source_index: 0,
        suite: None,
        name_truncated: length > 512,
        name_sanitized,
        diagnostic: None,
        diagnostic_truncated: false,
    }
}
fn diagnostic(case: &mut Case, text: &str) {
    let mut boundary = text.len().min(8192);
    while !text.is_char_boundary(boundary) {
        boundary -= 1;
    }
    case.diagnostic_truncated = text.len() > boundary;
    if !text.trim().is_empty() {
        case.diagnostic = Some(text[..boundary].trim().into());
    }
}
#[derive(Default)]
struct Parsed {
    format: &'static str,
    counts: Option<Counts>,
    cases: Vec<Case>,
    suites: usize,
    incomplete: bool,
    runner_failed: bool,
    warnings: Vec<String>,
}
impl Parsed {
    fn warning(&mut self, message: &str) {
        self.incomplete = true;
        if self.warnings.len() < 8 {
            self.warnings.push(message.into());
        }
    }
}
fn regex(pattern: &'static str) -> regex::Regex {
    regex::Regex::new(pattern).expect("constant report expression")
}
fn clean(text: &str) -> String {
    static ANSI: OnceLock<regex::Regex> = OnceLock::new();
    ANSI.get_or_init(|| regex(r"\x1b(?:\[[0-?]*[ -/]*[@-~]|\][^\x07\x1b]*(?:\x07|\x1b\\))"))
        .replace_all(text, "")
        .into_owned()
}
fn number(value: &Value, key: &str, required: bool) -> Result<u64, String> {
    match value.get(key) {
        None if !required => Ok(0),
        Some(n) => n.as_u64().filter(|n| *n <= MAX_COUNT).ok_or_else(|| {
            format!("Report {key} must be a nonnegative integer of at most one million.")
        }),
        _ => Err(format!("Report is missing {key}.")),
    }
}

fn parse_json(text: &str) -> Result<Option<Parsed>, String> {
    // npm and other wrappers can print a preamble before a JSON reporter.
    // Only objects beginning at a line boundary are considered, with a
    // bounded number of candidates and a recognized report schema.
    let mut found: Option<Parsed> = None;
    let mut consumed = 0;
    let mut candidates = 0;
    for offset in std::iter::once(0).chain(text.match_indices('\n').map(|(i, _)| i + 1)) {
        if offset < consumed {
            continue;
        }
        let slice = text[offset..].trim_start();
        if !slice.starts_with('{') {
            continue;
        }
        candidates += 1;
        if candidates > 16 {
            if let Some(found) = found.as_mut() {
                found.warning("JSON report candidate limit exceeded.");
            }
            break;
        }
        let mut stream = serde_json::Deserializer::from_str(slice).into_iter::<Value>();
        let Some(Ok(value)) = stream.next() else {
            continue;
        };
        consumed = offset + (text[offset..].len() - slice.len()) + stream.byte_offset();
        if value["schemaVersion"]
            .as_str()
            .is_some_and(|v| v.starts_with("locallm.check-report/") && v != VERSION)
        {
            return Err("Unsupported structured check report version.".into());
        }
        let mut result = Parsed::default();
        if value.get("numTotalTests").is_some() {
            result.format = "vitest-json";
            if value.get("success").is_some_and(|flag| !flag.is_boolean()) {
                return Err("JSON reporter success must be boolean.".into());
            }
            result.counts = Some(Counts::checked(
                number(&value, "numPassedTests", true)?,
                number(&value, "numFailedTests", true)?,
                number(&value, "numPendingTests", false)?,
                number(&value, "numTodoTests", false)?,
                Some(number(&value, "numTotalTests", true)?),
            )?);
            result.runner_failed = value.get("success") == Some(&Value::Bool(false))
                || number(&value, "numFailedTestSuites", false)? > 0;
            if let Some(files) = value.get("testResults").and_then(Value::as_array) {
                for (suite, file) in files.iter().enumerate() {
                    if let Some(cases) = file.get("assertionResults").and_then(Value::as_array) {
                        for item in cases {
                            let status = item["status"]
                                .as_str()
                                .ok_or("JSON assertion status is missing.")?;
                            let outcome = match status {
                                "passed" => "passed",
                                "failed" => "failed",
                                "pending" | "skipped" | "disabled" => "skipped",
                                "todo" => "todo",
                                _ => {
                                    return Err(format!(
                                        "Unknown JSON assertion status '{status}'."
                                    ))
                                }
                            };
                            let name = item["fullName"]
                                .as_str()
                                .or_else(|| item["title"].as_str())
                                .ok_or("JSON assertion name is missing.")?;
                            let duration = item
                                .get("duration")
                                .filter(|v| !v.is_null())
                                .map(|v| {
                                    v.as_f64().filter(|n| n.is_finite() && *n >= 0.0).ok_or(
                                        "JSON assertion duration must be finite and nonnegative.",
                                    )
                                })
                                .transpose()?;
                            let mut item_case = case(name, outcome, duration);
                            item_case.suite = Some(suite);
                            if let Some(messages) = item.get("failureMessages") {
                                let messages = messages
                                    .as_array()
                                    .ok_or("JSON failureMessages must be an array.")?
                                    .iter()
                                    .map(|s| {
                                        s.as_str()
                                            .ok_or("JSON failureMessages entries must be strings.")
                                    })
                                    .collect::<Result<Vec<_>, _>>()?;
                                diagnostic(&mut item_case, &messages.join("\n"));
                            }
                            result.cases.push(item_case);
                        }
                    }
                }
            }
        } else if value["schemaVersion"] == VERSION {
            result.format = "json";
            if value
                .get("runnerFailed")
                .is_some_and(|flag| !flag.is_boolean())
            {
                return Err("Structured runnerFailed must be boolean.".into());
            }
            let counts = value
                .get("counts")
                .ok_or("Structured check report is missing counts.")?;
            result.counts = Some(Counts::checked(
                number(counts, "passed", true)?,
                number(counts, "failed", true)?,
                number(counts, "skipped", false)?,
                number(counts, "todo", false)?,
                Some(number(counts, "total", true)?),
            )?);
            result.runner_failed = value.get("runnerFailed") == Some(&Value::Bool(true));
            if let Some(cases) = value.get("cases") {
                for item in cases
                    .as_array()
                    .ok_or("Structured cases must be an array.")?
                {
                    let outcome = item["outcome"]
                        .as_str()
                        .filter(|s| matches!(*s, "passed" | "failed" | "skipped" | "todo"))
                        .ok_or("Structured case outcome is invalid.")?;
                    let duration = item
                        .get("durationMs")
                        .filter(|v| !v.is_null())
                        .map(|v| {
                            v.as_f64()
                                .filter(|n| n.is_finite() && *n >= 0.0)
                                .ok_or("Structured durationMs must be finite and nonnegative.")
                        })
                        .transpose()?;
                    let mut item_case = case(
                        item["name"]
                            .as_str()
                            .ok_or("Structured case name is missing.")?,
                        outcome,
                        duration,
                    );
                    if let Some(message) = item.get("diagnostic") {
                        diagnostic(
                            &mut item_case,
                            message
                                .as_str()
                                .ok_or("Structured diagnostic must be text.")?,
                        );
                    }
                    result.cases.push(item_case);
                }
            }
        } else {
            continue;
        }
        result.suites = 1;
        let counts = result.counts.as_ref().unwrap();
        if result.cases.len() as u64 > counts.total {
            return Err("JSON report includes more cases than its total.".into());
        }
        for (outcome, declared) in [
            ("passed", counts.passed),
            ("failed", counts.failed),
            ("skipped", counts.skipped),
            ("todo", counts.todo),
        ] {
            if result
                .cases
                .iter()
                .filter(|case| case.outcome == outcome)
                .count() as u64
                > declared
            {
                return Err(format!(
                    "JSON case outcomes exceed the declared {outcome} count."
                ));
            }
        }
        if let Some(first) = found.as_mut() {
            first.runner_failed |=
                result.runner_failed || result.counts.as_ref().is_some_and(|c| c.failed > 0);
            first.warning("Multiple JSON reports were captured; counts and details describe the first report, not the complete command.");
            first.suites += 1;
        } else {
            found = Some(result);
        }
    }
    Ok(found)
}

fn parse_cargo(text: &str) -> Result<Option<Parsed>, String> {
    static SUMMARY: OnceLock<regex::Regex> = OnceLock::new();
    static CASE: OnceLock<regex::Regex> = OnceLock::new();
    static DIAGNOSTIC: OnceLock<regex::Regex> = OnceLock::new();
    let summary = SUMMARY.get_or_init(|| {
        regex(r"^test result: (ok|FAILED)\. (\d+) passed; (\d+) failed; (\d+) ignored;")
    });
    let test = CASE.get_or_init(|| regex(r"^test (.+) \.\.\. (ok|FAILED|ignored)(?:\b|$)"));
    let header = DIAGNOSTIC.get_or_init(|| regex(r"^---- (.+) stdout ----$"));
    let mut result = Parsed {
        format: "cargo",
        ..Default::default()
    };
    let mut counts = Counts::default();
    for line in text.lines() {
        if let Some(parts) = summary.captures(line) {
            let n = |i| {
                parts
                    .get(i)
                    .unwrap()
                    .as_str()
                    .parse::<u64>()
                    .map_err(|_| "Invalid Cargo test counts.".to_string())
            };
            counts.add(&Counts::checked(n(2)?, n(3)?, n(4)?, 0, None)?)?;
            result.runner_failed |= &parts[1] == "FAILED";
            result.suites += 1;
        } else if let Some(parts) = test.captures(line) {
            let outcome = match &parts[2] {
                "ok" => "passed",
                "FAILED" => "failed",
                _ => "skipped",
            };
            result.runner_failed |= outcome == "failed";
            let mut item = case(&parts[1], outcome, None);
            item.suite = Some(result.suites);
            result.cases.push(item);
        }
    }
    if result.suites == 0 {
        if result.cases.is_empty() {
            return Ok(None);
        }
        result.warning("Cargo case lines were captured without a final suite summary.");
    } else {
        result.counts = Some(counts);
    }
    let mut active: Option<usize> = None;
    let mut excerpt = String::new();
    let mut suite = 0;
    for line in text.lines() {
        let heading = header.captures(line);
        if heading.is_some() || line == "failures:" || line.starts_with("test result:") {
            if let Some(index) = active.take() {
                diagnostic(&mut result.cases[index], &excerpt);
            }
            excerpt.clear();
            if let Some(heading) = heading {
                active = result.cases.iter().rposition(|case| {
                    case.suite == Some(suite)
                        && case.name == heading[1]
                        && case.outcome == "failed"
                        && case.diagnostic.is_none()
                });
            }
        } else if active.is_some() {
            // The aggregate source is already bounded, and each source byte
            // belongs to at most one case's diagnostic excerpt.
            excerpt.push_str(line);
            excerpt.push('\n');
        }
        if summary.is_match(line) {
            suite += 1;
        }
    }
    if let Some(index) = active {
        diagnostic(&mut result.cases[index], &excerpt);
    }
    Ok(Some(result))
}

fn parse_vitest(text: &str) -> Result<Option<Parsed>, String> {
    static LINE: OnceLock<regex::Regex> = OnceLock::new();
    static COUNT: OnceLock<regex::Regex> = OnceLock::new();
    static TOTAL: OnceLock<regex::Regex> = OnceLock::new();
    let line = LINE.get_or_init(|| regex(r"^\s*Tests\s+(.+)$"));
    let count = COUNT.get_or_init(|| regex(r"(\d+)\s+(passed|failed|skipped|todo)\b"));
    let total = TOTAL.get_or_init(|| regex(r"\((\d+)\)\s*$"));
    let mut result = Parsed {
        format: "vitest-text",
        ..Default::default()
    };
    let mut counts = Counts::default();
    for text in text.lines() {
        let Some(line) = line.captures(text) else {
            continue;
        };
        let mut suite = Counts::default();
        let mut found = false;
        let mut categories = HashSet::new();
        for part in count.captures_iter(&line[1]) {
            if !categories.insert(part[2].to_string()) {
                return Err("Vitest summary repeats a count category.".into());
            }
            let n = part[1]
                .parse::<u64>()
                .map_err(|_| "Invalid Vitest count.")?;
            found = true;
            match &part[2] {
                "passed" => suite.passed = n,
                "failed" => suite.failed = n,
                "skipped" => suite.skipped = n,
                _ => suite.todo = n,
            };
        }
        if !found {
            continue;
        }
        let declared = total
            .captures(&line[1])
            .map(|part| part[1].parse::<u64>().map_err(|_| "Invalid Vitest total."))
            .transpose()?;
        counts.add(&Counts::checked(
            suite.passed,
            suite.failed,
            suite.skipped,
            suite.todo,
            declared,
        )?)?;
        result.suites += 1;
    }
    if result.suites == 0 {
        Ok(None)
    } else {
        result.counts = Some(counts);
        Ok(Some(result))
    }
}

fn parse_tap(text: &str) -> Result<Option<Parsed>, String> {
    static PLAN: OnceLock<regex::Regex> = OnceLock::new();
    static CASE: OnceLock<regex::Regex> = OnceLock::new();
    static DIRECTIVE: OnceLock<regex::Regex> = OnceLock::new();
    let plan = PLAN.get_or_init(|| regex(r"^1\.\.(\d+)(?:\s+#.*)?\s*$"));
    let test = CASE.get_or_init(|| regex(r"^(not ok|ok)(?:\s+(\d+))?(?:\s+(.*))?$"));
    let directive = DIRECTIVE.get_or_init(|| regex(r"(?i)#\s*(SKIP|TODO)\b"));
    let mut result = Parsed {
        format: "tap",
        ..Default::default()
    };
    let mut counts = Counts::default();
    let mut expected = None;
    let mut numbers = HashSet::new();
    let mut recognized = false;
    for line in text.lines() {
        if line.starts_with(char::is_whitespace) {
            continue;
        } // Nested subtests do not count twice.
        if line == "TAP version 13" || line == "TAP version 14" {
            recognized = true;
        }
        if line.starts_with("Bail out!") {
            result.runner_failed = true;
            result.warning("TAP runner bailed out.");
            recognized = true;
        }
        if let Some(plan) = plan.captures(line) {
            let n = plan[1].parse::<u64>().map_err(|_| "Invalid TAP plan.")?;
            if n > MAX_COUNT {
                return Err("TAP plan exceeds one million tests.".into());
            }
            if expected.replace(n).is_some() {
                result.warning("Multiple top-level TAP plans cannot be merged reliably.");
            }
            recognized = true;
        } else if let Some(test) = test.captures(line) {
            recognized = true;
            let ordinal = test
                .get(2)
                .map(|n| {
                    n.as_str()
                        .parse::<u64>()
                        .map_err(|_| "Invalid TAP case number.")
                })
                .transpose()?
                .unwrap_or(counts.total + 1);
            if ordinal == 0 || !numbers.insert(ordinal) {
                result.warning("TAP case numbers are zero or duplicated.");
            }
            let name = test
                .get(3)
                .map(|m| m.as_str().trim_start_matches("- "))
                .unwrap_or("unnamed test");
            let outcome = if let Some(directive) = directive.captures(name) {
                if directive[1].eq_ignore_ascii_case("TODO") {
                    "todo"
                } else {
                    "skipped"
                }
            } else if &test[1] == "not ok" {
                "failed"
            } else {
                "passed"
            };
            match outcome {
                "passed" => counts.passed += 1,
                "failed" => counts.failed += 1,
                "skipped" => counts.skipped += 1,
                _ => counts.todo += 1,
            };
            counts.total += 1;
            let display = directive
                .find(name)
                .map(|marker| name[..marker.start()].trim())
                .unwrap_or(name);
            result.cases.push(case(display, outcome, None));
        }
    }
    if !recognized {
        return Ok(None);
    }
    if expected != Some(counts.total) || numbers.iter().any(|n| *n > expected.unwrap_or(0)) {
        result.warning("TAP plan is missing or does not match captured top-level cases.");
    }
    result.counts = Some(counts);
    result.suites = 1;
    Ok(Some(result))
}

fn parse(text: &str, format: Format) -> Result<Option<Parsed>, String> {
    match format {
        Format::Json => parse_json(text),
        Format::Cargo => parse_cargo(text),
        Format::Vitest => match parse_json(text)? {
            Some(parsed) if parsed.format == "vitest-json" => Ok(Some(parsed)),
            _ => parse_vitest(text),
        },
        Format::Tap => parse_tap(text),
        Format::Auto => {
            let mut found: Option<Parsed> = None;
            for parsed in [
                parse_json(text)?,
                parse_cargo(text)?,
                parse_vitest(text)?,
                parse_tap(text)?,
            ]
            .into_iter()
            .flatten()
            {
                if let Some(first) = found.as_mut() {
                    first.runner_failed |= parsed.runner_failed
                        || parsed.counts.as_ref().is_some_and(|c| c.failed > 0);
                    first.warning("Multiple report formats were captured; selected counts do not cover the complete command.");
                } else {
                    found = Some(parsed);
                }
            }
            Ok(found)
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RunArgs {
    command: String,
    cwd: Option<String>,
    timeout_seconds: Option<u64>,
    yield_ms: Option<u64>,
    #[serde(default)]
    format: Format,
    case_offset: Option<usize>,
    max_cases: Option<usize>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReportArgs {
    id: String,
    #[serde(default)]
    format: Format,
    wait_ms: Option<u64>,
    case_offset: Option<usize>,
    max_cases: Option<usize>,
    expected_sha256: Option<String>,
}

fn report(
    snapshot: Value,
    format: Format,
    offset: usize,
    limit: usize,
    expected: Option<&str>,
    started: bool,
) -> Result<Value, String> {
    if limit > MAX_CASES || offset > MAX_COUNT as usize {
        return Err("Use case_offset0–1000000 and max_cases0–200.".into());
    }
    let mut stdout = String::new();
    let mut stderr = String::new();
    for frame in snapshot["output"]
        .as_array()
        .ok_or("Process snapshot is missing output frames.")?
    {
        let target = if frame["stream"] == "stdout" {
            &mut stdout
        } else {
            &mut stderr
        };
        target.push_str(
            frame["text"]
                .as_str()
                .ok_or("Invalid process output frame.")?,
        );
    }
    let mut digest = Sha256::new();
    for part in [format.name(), stdout.as_str(), stderr.as_str()] {
        digest.update((part.len() as u64).to_le_bytes());
        digest.update(part.as_bytes());
    }
    let sha = format!("{:x}", digest.finalize());
    if expected.is_some_and(|pin| pin != sha) {
        return Err("Report output or parser format changed. Refresh the source hash before paging details.".into());
    }
    let stdout = clean(&stdout);
    let stderr = clean(&stderr);
    let parsed = parse(&stdout, format);
    let (parsed, source, parse_error) = match parsed {
        Ok(Some(parsed)) => (Some(parsed), "stdout", None),
        Err(error) => (None, "stdout", Some(error)),
        Ok(None) => match parse(&stderr, format) {
            Ok(Some(parsed)) => (Some(parsed), "stderr", None),
            Ok(None) => (None, "none", None),
            Err(error) => (None, "stderr", Some(error)),
        },
    };
    let mut parsed = parsed.unwrap_or_default();
    if source == "stdout" {
        match parse(&stderr, format) {
            Ok(Some(other)) => {
                parsed.runner_failed |=
                    other.runner_failed || other.counts.as_ref().is_some_and(|c| c.failed > 0);
                parsed.warning(
                    "Reports were captured on both streams; selected counts describe stdout only.",
                );
            }
            Err(error) => {
                parsed.warning(&format!("stderr report is invalid: {error}"));
            }
            _ => (),
        }
    }
    let log_loss = snapshot["truncated"] == true || snapshot["hasMore"] == true;
    let missing = snapshot["state"] != "running"
        && format != Format::Auto
        && parsed.counts.is_none()
        && parsed.cases.is_empty();
    let incomplete = log_loss || parsed.incomplete || parse_error.is_some() || missing;
    if log_loss {
        parsed.warnings.push(
            "Some process output was lost or unreadable; counts cover retained output only.".into(),
        );
    }
    if missing {
        parsed
            .warnings
            .push("No complete report matched the explicitly requested format.".into());
    }
    if let Some(error) = &parse_error {
        parsed.warnings.push(error.clone());
    }
    let state = snapshot["state"]
        .as_str()
        .ok_or("Process snapshot is missing state.")?;
    let exit = snapshot["exitCode"].as_i64();
    let count_failure = parsed.runner_failed
        || parsed.counts.as_ref().is_some_and(|c| c.failed > 0)
        || parsed.cases.iter().any(|c| c.outcome == "failed");
    let verdict = match state {
        "running" => "running",
        "timed_out" => "timed_out",
        "stopped" => "stopped",
        "failed" | "unknown" => "unknown",
        "exited" if exit.is_none() => "unknown",
        "exited" if count_failure => "failed",
        "exited" if exit != Some(0) => "command_failed",
        "exited" if incomplete => "incomplete",
        "exited"
            if parsed
                .counts
                .as_ref()
                .is_some_and(|c| c.passed == 0 && c.failed == 0) =>
        {
            "no_tests"
        }
        "exited" if parsed.counts.is_some() => "passed",
        "exited" => "command_succeeded",
        _ => "unknown",
    };
    let failed = !matches!(verdict, "running" | "passed" | "command_succeeded");
    let tests_passed = if count_failure {
        Some(false)
    } else if verdict == "passed" {
        Some(true)
    } else {
        None
    };
    let command_succeeded = match state {
        "exited" => exit.map(|code| code == 0),
        "timed_out" | "stopped" | "failed" => Some(false),
        _ => None,
    };
    let total_cases = parsed.cases.len();
    let details_complete = !incomplete
        && parsed
            .counts
            .as_ref()
            .is_some_and(|c| c.total == total_cases as u64);
    // Stable order keeps failing cases useful even when details are paged.
    for (index, case) in parsed.cases.iter_mut().enumerate() {
        case.source_index = index;
    }
    parsed
        .cases
        .sort_by_key(|case| match case.outcome.as_str() {
            "failed" => 0,
            "todo" => 1,
            "skipped" => 2,
            _ => 3,
        });
    let has_more = offset.saturating_add(limit) < total_cases;
    let mut process = snapshot;
    process
        .as_object_mut()
        .ok_or("Invalid process snapshot.")?
        .remove("output");
    let message=match verdict {
        "passed"=>"Reported tests passed and the command exited successfully.",
        "command_succeeded"=>"Command exited successfully. No recognized test report was found; test counts and test success are unknown.",
        "running"=>"Checks are still running. Poll check_report with this process ID.",
        "no_tests"=>"No executed passing tests were reported. Zero tests or entirely skipped/todo cases do not verify that tests passed.",
        "incomplete"=>"Report evidence is incomplete; no full-suite pass is claimed.",
        "command_failed"=>"The command exited unsuccessfully. Inspect the report and process output before correcting the failure.",
        "failed"=>"The report contains test or runner failures, even if the command returned zero.",
        "timed_out"=>"Checks timed out. The command may have changed files; do not blindly replay it.",
        "stopped"=>"Checks were stopped before a successful completion was confirmed.",
        _=>"Check completion is unknown; do not assume the command or its side effects completed.",
    };
    let mut result = json!({"processId":process["id"],"started":started,"process":process,"report":{
        "schemaVersion":VERSION,"format":if parsed.format.is_empty(){"unrecognized"}else{parsed.format},"requestedFormat":format.name(),
        "verdict":verdict,"commandSucceeded":command_succeeded,"testsPassed":tests_passed,"counts":parsed.counts,"suites":parsed.suites,"runnerFailed":parsed.runner_failed,
        "incomplete":incomplete,"countsScope":if log_loss{"retained_output"}else{"selected_reports"},"sourceStream":source,"sourceSha256":sha,"caseDetailsComplete":details_complete,
        "caseOffset":offset,"caseOrder":"failures_first_then_source_order","cases":parsed.cases.into_iter().skip(offset).take(limit).collect::<Vec<_>>(),"availableCases":total_cases,
        "hasMoreCases":has_more,"nextCaseOffset":if has_more&&limit>0{Some(offset+limit)}else{None},"warnings":parsed.warnings
    },"isError":failed && !started,"retryable":false,"sideEffects":if started{"unknown"}else{"none"},"message":message});
    if failed && !started {
        result["code"] = json!(format!("checks-{verdict}"));
    }
    let mut tail = stderr.len().saturating_sub(8192);
    while !stderr.is_char_boundary(tail) {
        tail += 1;
    }
    result["report"]["stderrExcerpt"] = json!(&stderr[tail..]);
    result["report"]["stderrExcerptTruncated"] = json!(tail > 0);
    Ok(result)
}

pub(crate) fn effective_config(
    mut config: crate::execution::ExecutionConfig,
    cwd: Option<&str>,
) -> crate::execution::ExecutionConfig {
    for (key, value) in [
        ("CI", "1"),
        ("NO_COLOR", "1"),
        ("CARGO_TERM_COLOR", "never"),
    ] {
        config
            .bash
            .environment
            .entry(key.into())
            .or_insert(value.into());
    }
    if let Some(cwd) = cwd {
        config.bash.working_directory = cwd.into();
    }
    config
}
pub async fn execute(
    registry: &crate::process_tools::ProcessRegistry,
    owner: &str,
    workspace: &str,
    config: crate::execution::ExecutionConfig,
    alias: &str,
    args: Value,
) -> Result<Value, String> {
    execute_observed(registry, owner, workspace, config, alias, args, None).await
}
pub(crate) async fn execute_observed(
    registry: &crate::process_tools::ProcessRegistry,
    owner: &str,
    workspace: &str,
    mut config: crate::execution::ExecutionConfig,
    alias: &str,
    args: Value,
    completion: Option<crate::verification_observer::Completion>,
) -> Result<Value, String> {
    match alias {
        "check_run" => {
            let args: RunArgs = serde_json::from_value(args).map_err(|e| e.to_string())?;
            let limit = args.max_cases.unwrap_or(50);
            let offset = args.case_offset.unwrap_or(0);
            if limit > MAX_CASES || offset > MAX_COUNT as usize {
                return Err("Invalid report case bounds; command was not launched.".into());
            }
            config = effective_config(config, args.cwd.as_deref());
            let mut start = json!({"command":args.command,"yield_ms":args.yield_ms.unwrap_or(100)});
            if let Some(cwd) = args.cwd {
                start["cwd"] = json!(cwd);
            }
            if let Some(timeout) = args.timeout_seconds {
                start["timeout_seconds"] = json!(timeout);
            }
            if let Some(notice) = crate::process_tools::start_limit_notice(&config, &start) { return Ok(notice); }
            let started = registry
                .start_observed(owner, workspace, config, start, completion)
                .await?;
            let id = started["id"]
                .as_str()
                .ok_or("Process start returned no ID.")?;
            report(
                registry.retained_output(owner, workspace, id)?,
                args.format,
                offset,
                limit,
                None,
                true,
            )
        }
        "check_report" => {
            let args: ReportArgs = serde_json::from_value(args).map_err(|e| e.to_string())?;
            if args.id.starts_with("dev_") {
                return Ok(crate::tool_envelope::failure_notice(
                    "wrong-id-type", "id requires a processId (proc-…) returned by check_run or verification_run. An evidenceId from verification_run belongs in verification_report({evidence_id: evidenceId}). A reportId belongs in verification_compare. No check was rerun.",
                    false, crate::tool_envelope::SIDE_NONE, &json!({"field":"/id"}),
                ));
            }
            let wait = args.wait_ms.unwrap_or(0);
            if wait > 1000 {
                return Err("Report wait_ms must be0–1000.".into());
            }
            // Check ownership before any delay; the snapshot rechecks it too.
            registry.retained_output(owner, workspace, &args.id)?;
            if wait > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(wait)).await;
            }
            let snapshot = registry.retained_output(owner, workspace, &args.id)?;
            // Parsing a large report is read-only CPU work. Offload it so
            // parallel read orchestration can overlap without blocking the
            // async executor; cancellation never restarts the owned command.
            tokio::task::spawn_blocking(move || {
                report(
                    snapshot,
                    args.format,
                    args.case_offset.unwrap_or(0),
                    args.max_cases.unwrap_or(50),
                    args.expected_sha256.as_deref(),
                    false,
                )
            })
            .await
            .map_err(|_| "Check report worker failed; the command was not restarted.".to_string())?
        }
        _ => Err("Unknown check tool.".into()),
    }
}

pub fn registry() -> Vec<crate::tool_spec::ToolSpec> {
    let schema = |properties, required: &[&str]| json!({"type":"object","properties":properties,"required":required,"additionalProperties":false});
    let format = json!({"type":"string","enum":["auto","cargo","vitest","tap","json"],"default":"auto","description":"Recognized report format. JSON accepts Vitest/Jest summaries or locallm.check-report/1 counts/cases. Explicit formats fail loudly when missing or invalid; auto retains an honest command-only outcome."});
    let offset = json!({"type":"integer","minimum":0,"maximum":1000000,"default":0});
    let limit = json!({"type":"integer","minimum":0,"maximum":200,"default":50,"description":"Case detail page size;0 returns summary only. Failing cases sort first, then source order."});
    vec![
        ("check_run".into(),"Run a build, test suite or check with the configured approved shell. Returns an owned process ID and structured report promptly; use check_report for completion, process_stop for cancellation, and process_poll for raw output. Launch success does not mean tests passed: inspect report.verdict/testsPassed/counts. No implicit command retry. Uses configured environment/time/memory/approval limits; CI=1/NO_COLOR/CARGO_TERM_COLOR=never defaults respect explicit saved environment overrides.".into(),schema(json!({"command":{"type":"string","minLength":1,"maxLength":32768},"cwd":{"type":"string","maxLength":4096},"timeout_seconds":{"type":"integer","minimum":1,"maximum":300},"yield_ms":{"type":"integer","minimum":0,"maximum":1000,"default":100},"format":format,"case_offset":offset,"max_cases":limit}),&["command"])),
        ("check_report".into(),"Read structured build/test evidence from a conversation/workspace-owned process without rerunning it. Parses Cargo/libtest, Vitest/Jest JSON or text, TAP, or locallm.check-report/1 JSON. Reports failed commands, zero tests, missing/invalid reports, timeout, stop, unknown outcomes and log loss explicitly. A successful exit without recognized tests never claims testsPassed. Page case details with sourceSha256 pin; inspect raw output via process_poll.".into(),schema(json!({"id":{"type":"string","minLength":1,"maxLength":128},"format":format,"wait_ms":{"type":"integer","minimum":0,"maximum":1000,"default":0},"case_offset":offset,"max_cases":limit,"expected_sha256":{"type":"string","pattern":"^[a-f0-9]{64}$"}}),&["id"])),
    ].into_iter().map(crate::tool_spec::ToolSpec::from).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn evidence_id_is_not_a_process_and_inspection_never_launches_a_command() {
        let registry = crate::process_tools::ProcessRegistry::default();
        let result = execute(&registry, "owner", "/fixture", Default::default(), "check_report", json!({"id":"dev_evidence"})).await.unwrap();
        assert_eq!(result["error"]["code"], "wrong-id-type");
        assert_eq!(result["error"]["sideEffects"], "none");
        assert!(!registry.has_owned("owner", "/fixture").unwrap());
    }
    fn snapshot(stdout: &str, stderr: &str, exit: Option<i32>, state: &str) -> Value {
        json!({"id":"fixture-process","state":state,"exitCode":exit,"error":null,"output":[{"cursor":0,"stream":"stdout","text":stdout},{"cursor":1,"stream":"stderr","text":stderr}],"hasMore":false,"truncated":false,"firstAvailableCursor":0,"nextCursor":2,"sideEffects":"possible"})
    }
    fn inspect(stdout: &str, stderr: &str, exit: i32, format: Format) -> Value {
        report(
            snapshot(stdout, stderr, Some(exit), "exited"),
            format,
            0,
            50,
            None,
            false,
        )
        .unwrap()
    }
    fn structured(passed: u64, failed: u64) -> String {
        json!({"schemaVersion":VERSION,"counts":{"total":passed+failed,"passed":passed,"failed":failed,"skipped":0,"todo":0}}).to_string()
    }

    #[test]
    fn cargo_suites_aggregate_without_mixing_same_named_failure_diagnostics() {
        let log="running 2 tests\ntest good ... ok\ntest same::name ... FAILED\n\nfailures:\n---- same::name stdout ----\nfirst failure detail\n\nfailures:\n same::name\ntest result: FAILED. 1 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in0.01s\nrunning 1 test\ntest same::name ... FAILED\nfailures:\n---- same::name stdout ----\nsecond failure detail\nfailures:\n same::name\ntest result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out;\n";
        let result = inspect(log, "", 101, Format::Auto);
        assert_eq!(
            result["report"]["counts"],
            json!({"total":3,"passed":1,"failed":2,"skipped":0,"todo":0})
        );
        assert_eq!(result["report"]["suites"], 2);
        assert_eq!(result["report"]["verdict"], "failed");
        assert_eq!(result["report"]["testsPassed"], false);
        assert_eq!(result["isError"], true);
        assert!(result["report"]["cases"][0]["diagnostic"]
            .as_str()
            .unwrap()
            .contains("first failure"));
        assert!(result["report"]["cases"][1]["diagnostic"]
            .as_str()
            .unwrap()
            .contains("second failure"));
        assert_eq!(result["report"]["cases"][0]["suite"], 0);
        assert_eq!(result["report"]["cases"][1]["suite"], 1);
        assert_eq!(result["report"]["cases"][2]["outcome"], "passed");
    }

    #[test]
    fn vitest_json_and_text_preserve_counts_names_durations_and_failure_evidence() {
        let value = json!({"numTotalTests":5,"numPassedTests":2,"numFailedTests":1,"numPendingTests":1,"numTodoTests":1,"numFailedTestSuites":1,"success":false,"testResults":[{"assertionResults":[
            {"fullName":"normal","status":"passed","duration":0.5},
            {"fullName":"long 🦀 ".repeat(200),"status":"failed","duration":2.5,"failureMessages":["assertion","stack".repeat(4000)]},
            {"fullName":"other","status":"passed"},{"fullName":"skip","status":"pending"},{"fullName":"todo","status":"todo"}
        ]}]});
        let output = format!(
            "> npm wrapper\n{}\n",
            serde_json::to_string_pretty(&value).unwrap()
        );
        let result = inspect(&output, "", 0, Format::Vitest);
        assert_eq!(result["report"]["counts"]["total"], 5);
        assert_eq!(
            result["report"]["verdict"], "failed",
            "A runner's zero exit cannot hide failed tests"
        );
        assert_eq!(result["report"]["cases"][0]["nameTruncated"], true);
        assert_eq!(result["report"]["cases"][0]["diagnosticTruncated"], true);
        assert_eq!(result["report"]["cases"][0]["durationMs"], 2.5);
        assert_eq!(result["report"]["cases"][0]["sourceIndex"], 1);
        let result = inspect(
            "\u{1b}[32m Tests 2 passed | 1 skipped (3)\u{1b}[0m\n",
            "",
            0,
            Format::Auto,
        );
        assert_eq!(result["report"]["verdict"], "passed");
        assert_eq!(
            result["report"]["caseDetailsComplete"], false,
            "Text summary did not supply case details"
        );
        assert_eq!(result["report"]["counts"]["skipped"], 1);
    }

    #[test]
    fn tap_handles_nested_cases_skip_todo_bailouts_and_incomplete_plans() {
        let result=inspect("TAP version 13\n    not ok1 nested child\nok 1 - parent\nok 2 - missing # SKIP environment\nnot ok 3 - future # TODO expected\n1..3\n","",0,Format::Tap);
        assert_eq!(
            result["report"]["counts"],
            json!({"total":3,"passed":1,"failed":0,"skipped":1,"todo":1})
        );
        assert_eq!(result["report"]["verdict"], "passed");
        assert_eq!(
            inspect("TAP version 13\n1..0 # SKIP none\n", "", 0, Format::Tap)["report"]["verdict"],
            "no_tests"
        );
        assert_eq!(
            inspect(
                "TAP version 13\nok 1 - first\nBail out! broken\n",
                "",
                0,
                Format::Tap
            )["report"]["verdict"],
            "failed"
        );
        for log in [
            "ok 1 - a\nok 1 - duplicate\n1..2\n",
            "ok 1 - a\n1..2\n",
            "ok 1 - a\n",
        ] {
            assert_eq!(
                inspect(log, "", 0, Format::Tap)["report"]["verdict"],
                "incomplete",
                "{log}"
            );
        }
    }

    #[test]
    fn command_success_is_not_test_success_and_missing_or_invalid_reports_fail_loudly() {
        let result = inspect("Build complete\n", "", 0, Format::Auto);
        assert_eq!(result["report"]["verdict"], "command_succeeded");
        assert!(result["report"]["testsPassed"].is_null());
        assert!(result["report"]["counts"].is_null());
        for log in [
            "TAP version 13\nok 1 - unavailable # SKIP environment\nnot ok 2 - future # TODO pending\n1..2\n",
            "test result: ok. 0 passed; 0 failed; 5 ignored;\n",
            " Tests 2 skipped | 1 todo (3)\n",
        ] {
            let skipped=inspect(log,"",0,Format::Auto);
            assert_eq!(skipped["report"]["verdict"],"no_tests","{skipped}");
            assert!(skipped["report"]["testsPassed"].is_null());
            assert_eq!(skipped["report"]["commandSucceeded"],true);
        }
        assert_eq!(
            inspect("Build complete", "", 0, Format::Cargo)["report"]["verdict"],
            "incomplete"
        );
        assert_eq!(
            inspect("", "compiler failed", 2, Format::Auto)["report"]["verdict"],
            "command_failed"
        );
        for value in [
            json!({"schemaVersion":"locallm.check-report/999","counts":{"total":1,"passed":1,"failed":0}}),
            json!({"schemaVersion":VERSION,"counts":{"total":2,"passed":1,"failed":0}}),
            json!({"numTotalTests":1,"numPassedTests":-1,"numFailedTests":0}),
            json!({"numTotalTests":1,"numPassedTests":1,"numFailedTests":0,"success":"false"}),
        ] {
            let result = inspect(&value.to_string(), "", 0, Format::Auto);
            assert_eq!(result["report"]["verdict"], "incomplete", "{result}");
            assert_eq!(result["isError"], true);
            assert!(!result["report"]["warnings"].as_array().unwrap().is_empty());
        }
    }

    #[test]
    fn multiple_reports_or_streams_never_hide_a_failure_or_claim_complete_totals() {
        let pass = structured(1, 0);
        let failure = structured(0, 1);
        let result = inspect(&format!("{pass}\n{failure}\n"), "", 0, Format::Auto);
        assert_eq!(result["report"]["verdict"], "failed");
        assert_eq!(result["report"]["incomplete"], true);
        let result = inspect(&pass, &failure, 0, Format::Auto);
        assert_eq!(result["report"]["verdict"], "failed");
        let cargo = "test result: ok. 1 passed; 0 failed; 0 ignored;\n";
        let tap = "TAP version 13\nnot ok 1 - failure\n1..1\n";
        assert_eq!(
            inspect(&format!("{cargo}{tap}"), "", 0, Format::Auto)["report"]["verdict"],
            "failed"
        );
        let result = inspect(&format!("{pass}\n{pass}"), "", 0, Format::Auto);
        assert_eq!(
            result["report"]["verdict"], "incomplete",
            "Duplicate/multiple output must not silently double counts"
        );
    }

    #[test]
    fn output_loss_case_paging_and_source_pins_are_explicit() {
        let output = "TAP version 13\nok 1 - alpha\nnot ok 2 - beta\nok 3 - gamma\n1..3\n";
        let first = report(
            snapshot(output, "", Some(0), "exited"),
            Format::Auto,
            0,
            1,
            None,
            false,
        )
        .unwrap();
        assert_eq!(first["report"]["cases"][0]["name"], "beta");
        assert_eq!(first["report"]["hasMoreCases"], true);
        let pin = first["report"]["sourceSha256"].as_str().unwrap();
        let next = report(
            snapshot(output, "", Some(0), "exited"),
            Format::Auto,
            1,
            1,
            Some(pin),
            false,
        )
        .unwrap();
        assert_eq!(next["report"]["cases"][0]["name"], "alpha");
        assert!(report(
            snapshot(&format!("{output}more\n"), "", Some(0), "exited"),
            Format::Auto,
            1,
            1,
            Some(pin),
            false
        )
        .is_err());
        assert!(report(
            snapshot(output, "", Some(0), "exited"),
            Format::Tap,
            1,
            1,
            Some(pin),
            false
        )
        .is_err());
        let mut lost = snapshot(
            "test result: ok. 5 passed; 0 failed; 0 ignored;\n",
            "",
            Some(0),
            "exited",
        );
        lost["truncated"] = json!(true);
        lost["firstAvailableCursor"] = json!(99);
        let result = report(lost, Format::Auto, 0, 50, None, false).unwrap();
        assert_eq!(result["report"]["verdict"], "incomplete");
        assert_eq!(result["report"]["countsScope"], "retained_output");
        assert!(result["report"]["testsPassed"].is_null());
        let summary = report(
            snapshot(output, "", Some(0), "exited"),
            Format::Auto,
            0,
            0,
            None,
            false,
        )
        .unwrap();
        assert!(summary["report"]["cases"].as_array().unwrap().is_empty());
        assert!(summary["report"]["nextCaseOffset"].is_null());
    }

    async fn finished(
        registry: &crate::process_tools::ProcessRegistry,
        root: &str,
        id: &str,
        format: &str,
    ) -> Value {
        tokio::time::timeout(std::time::Duration::from_secs(15), async {
            loop {
                let value = execute(
                    registry,
                    "owner",
                    root,
                    crate::execution::ExecutionConfig::default(),
                    "check_report",
                    json!({"id":id,"format":format,"wait_ms":20}),
                )
                .await
                .unwrap();
                if value["process"]["state"] != "running" {
                    return value;
                }
            }
        })
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn owned_process_reports_failures_even_with_zero_exit_and_support_stop_and_timeout() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().to_str().unwrap();
        let registry = crate::process_tools::ProcessRegistry::default();
        let command = if cfg!(windows) {
            "Write-Output 'TAP version 13'; Write-Output '1..2'; Write-Output 'ok 1 - good'; Write-Output 'not ok 2 - bad'; exit 0"
        } else {
            "printf 'TAP version 13\\n1..2\\nok 1 - good\\nnot ok 2 - bad\\n'; exit 0"
        };
        let run = execute(
            &registry,
            "owner",
            path,
            crate::execution::ExecutionConfig::default(),
            "check_run",
            json!({"command":command,"yield_ms":0}),
        )
        .await
        .unwrap();
        assert_eq!(run["started"], true);
        assert_eq!(
            run["isError"], false,
            "Launch success is separate from completed test success"
        );
        let id = run["processId"].as_str().unwrap();
        let report = finished(&registry, path, id, "auto").await;
        assert_eq!(report["report"]["verdict"], "failed", "{report}");
        assert_eq!(report["process"]["exitCode"], 0);
        assert_eq!(report["isError"], true);
        assert!(execute(
            &registry,
            "intruder",
            path,
            crate::execution::ExecutionConfig::default(),
            "check_report",
            json!({"id":id})
        )
        .await
        .is_err());
        assert!(execute(
            &registry,
            "owner",
            "/different-workspace",
            crate::execution::ExecutionConfig::default(),
            "check_report",
            json!({"id":id})
        )
        .await
        .is_err());
        let sleep = if cfg!(windows) {
            "Start-Sleep -Seconds 30"
        } else {
            "sleep 30"
        };
        let stopped = execute(
            &registry,
            "owner",
            path,
            crate::execution::ExecutionConfig::default(),
            "check_run",
            json!({"command":sleep,"yield_ms":0}),
        )
        .await
        .unwrap();
        let id = stopped["processId"].as_str().unwrap();
        registry
            .stop("owner", path, json!({"id":id}))
            .await
            .unwrap();
        assert_eq!(
            finished(&registry, path, id, "auto").await["report"]["verdict"],
            "stopped"
        );
        let timeout = execute(
            &registry,
            "owner",
            path,
            crate::execution::ExecutionConfig::default(),
            "check_run",
            json!({"command":sleep,"timeout_seconds":1,"yield_ms":0}),
        )
        .await
        .unwrap();
        let result = finished(
            &registry,
            path,
            timeout["processId"].as_str().unwrap(),
            "auto",
        )
        .await;
        assert_eq!(result["report"]["verdict"], "timed_out", "{result}");
        assert!(result["report"]["testsPassed"].is_null());
    }

    #[tokio::test]
    async fn cancelled_undelivered_check_start_does_not_leave_an_owned_process() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().to_str().unwrap().to_string();
        let registry = std::sync::Arc::new(crate::process_tools::ProcessRegistry::default());
        let cloned = registry.clone();
        let owned = path.clone();
        let future = tokio::spawn(async move {
            execute(&cloned,"owner",&owned,crate::execution::ExecutionConfig::default(),"check_run",json!({"command":if cfg!(windows){"Start-Sleep -Seconds 30"}else{"sleep 30"},"yield_ms":1000})).await
        });
        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            while !registry.has_owned("owner", &path).unwrap() {
                tokio::time::sleep(std::time::Duration::from_millis(2)).await;
            }
        })
        .await
        .unwrap();
        future.abort();
        assert!(future.await.unwrap_err().is_cancelled());
        assert!(!registry.has_owned("owner", &path).unwrap());
    }

    #[tokio::test]
    async fn actual_cargo_test_project_yields_structured_cases_and_assertion_diagnostic() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("src")).unwrap();
        std::fs::write(
            root.path().join("Cargo.toml"),
            "[package]\nname='native-check-fixture'\nversion='0.0.0'\nedition='2021'\n",
        )
        .unwrap();
        std::fs::write(root.path().join("src/lib.rs"),"#[cfg(test)] mod tests { #[test] fn good(){assert_eq!(2+2,4);} #[test] fn bad(){assert_eq!(1,2);} #[test] #[ignore] fn skipped(){} }\n").unwrap();
        let cargo = env!("CARGO");
        let command = if cfg!(windows) {
            format!(
                "& '{}' test --offline --color never -- --test-threads=1",
                cargo.replace('\'', "''")
            )
        } else {
            format!(
                "'{}' test --offline --color never -- --test-threads=1",
                cargo.replace('\'', "'\\''")
            )
        };
        let registry = crate::process_tools::ProcessRegistry::default();
        let path = root.path().to_str().unwrap();
        let started = execute(
            &registry,
            "owner",
            path,
            crate::execution::ExecutionConfig::default(),
            "check_run",
            json!({"command":command,"yield_ms":0,"timeout_seconds":30}),
        )
        .await
        .unwrap();
        let result = finished(
            &registry,
            path,
            started["processId"].as_str().unwrap(),
            "cargo",
        )
        .await;
        assert_eq!(
            result["report"]["counts"],
            json!({"total":3,"passed":1,"failed":1,"skipped":1,"todo":0}),
            "{result}"
        );
        assert_eq!(result["report"]["cases"][0]["name"], "tests::bad");
        assert!(
            result["report"]["cases"][0]["diagnostic"]
                .as_str()
                .unwrap()
                .contains("assertion"),
            "{result}"
        );
        assert_eq!(result["report"]["testsPassed"], false);
    }

    #[tokio::test]
    async fn actual_retention_loss_never_becomes_a_full_suite_pass() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().to_str().unwrap();
        let command = if cfg!(windows) {
            "Write-Output ('x'*350000); Write-Output 'TAP version 13'; Write-Output '1..1'; Write-Output 'ok 1 - last'"
        } else {
            "head -c 350000 /dev/zero | tr '\\0' x; printf '\\nTAP version 13\\n1..1\\nok 1 - last\\n'"
        };
        let registry = crate::process_tools::ProcessRegistry::default();
        let run = execute(
            &registry,
            "owner",
            path,
            crate::execution::ExecutionConfig::default(),
            "check_run",
            json!({"command":command,"yield_ms":0}),
        )
        .await
        .unwrap();
        let result = finished(&registry, path, run["processId"].as_str().unwrap(), "auto").await;
        assert_eq!(result["process"]["exitCode"], 0);
        assert_eq!(result["report"]["commandSucceeded"], true);
        assert_eq!(result["report"]["verdict"], "incomplete", "{result}");
        assert!(result["report"]["testsPassed"].is_null());
        assert!(result["process"]["firstAvailableCursor"].as_u64().unwrap() > 0);
    }

    #[tokio::test]
    async fn public_schema_model_call_dispatch_source_approval_and_capability_gates() {
        let root = tempfile::tempdir().unwrap();
        let state = crate::test_support::app_state(root.path());
        crate::test_support::select_fixture_model(&state, root.path());
        let conversation = state.database().unwrap().create_conversation().unwrap().id;
        let mut config = state.database().unwrap().execution_config().unwrap();
        config.bash.always_ask = true;
        state
            .database()
            .unwrap()
            .save_execution_config(&config)
            .unwrap();
        let plan = crate::chat::assemble_turn(
            &state,
            &conversation,
            "Run checks",
            vec!["__execution".into()],
            Some(vec![]),
            Some(crate::presets::STANDARD),
            None,
        )
        .await
        .unwrap();
        let run = plan
            .tools
            .iter()
            .find(|tool| tool.alias == "check_run")
            .unwrap();
        let reader = plan
            .tools
            .iter()
            .find(|tool| tool.alias == "check_report")
            .unwrap();
        assert!(run.requires_approval(&json!({})));
        assert!(!run.trusted_read());
        assert!(reader.trusted_read());
        assert!(!reader.requires_approval(&json!({})));
        for tool in [run, reader] {
            assert_eq!(
                tool.definition()["function"]["parameters"]["additionalProperties"],
                false
            );
        }
        for invalid in [
            json!({"command":"anything","extra":true}),
            json!({"command":"anything","max_cases":201}),
            json!({"command":"anything","format":"unknown"}),
        ] {
            assert!(run.prepare_arguments(invalid).is_err());
        }
        assert!(reader
            .prepare_arguments(json!({"id":"fixture","expected_sha256":"invalid"}))
            .is_err());
        // Execute only after selecting FullAccess with always-ask disabled;
        // the separate assembly assertions above prove that saved override.
        config.bash.always_ask = false;
        state
            .database()
            .unwrap()
            .save_execution_config(&config)
            .unwrap();
        let tools = ["check_run", "check_report"]
            .into_iter()
            .map(|name| crate::connectors::AgentTool::harness(name).unwrap())
            .collect::<Vec<_>>();
        let ctx = crate::harness::HarnessCtx {
            state: &state,
            snapshot: crate::test_support::snapshot(
                &state,
                root.path(),
                &["check_run", "check_report"],
            ),
            conversation_id: conversation.clone(),
            run_id: "checks-fixture".into(),
            step_id: "step0".into(),
            access_mode: crate::permissions::AccessMode::FullAccess,
            depth: 0,
            preset_id: crate::presets::STANDARD.into(),
            inherit_tools: &tools,
            emit: std::sync::Arc::new(|_, _| {}),
        };
        let command = if cfg!(windows) {
            "Write-Output 'TAP version 13'; Write-Output '1..1'; Write-Output 'ok 1 - native schema fixture'"
        } else {
            "printf 'TAP version 13\\n1..1\\nok 1 - native schema fixture\\n'"
        };
        let mut calls = crate::tool_calls::ToolCalls::default();
        calls.push(&json!({"tool_calls":[{"index":0,"id":"checks-call","type":"function","function":{"name":"check_run","arguments":json!({"command":command,"yield_ms":0}).to_string()}}]})).unwrap();
        let call = calls.finish().unwrap().remove(0);
        let tool = tools.iter().find(|tool| tool.alias == call.name).unwrap();
        assert!(ctx
            .access_mode
            .automatic_reason(tool.trusted_read())
            .is_some());
        assert!(!tool.requires_approval(&call.arguments));
        let started = crate::harness::execute(
            &ctx,
            &call.name,
            tool.prepare_arguments(call.arguments).unwrap(),
        )
        .await
        .unwrap();
        let id = started.value["processId"].as_str().unwrap();
        let result = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let tool = &tools[1];
                let outcome = crate::harness::execute(
                    &ctx,
                    "check_report",
                    tool.prepare_arguments(json!({"id":id,"wait_ms":20}))
                        .unwrap(),
                )
                .await
                .unwrap();
                if outcome.value["process"]["state"] != "running" {
                    break outcome.value;
                }
            }
        })
        .await
        .unwrap();
        assert_eq!(result["report"]["testsPassed"], true, "{result}");
        let normalized = crate::tool_envelope::normalize(result, reader.envelope_family());
        assert_eq!(
            crate::tool_envelope::validate_envelope(&normalized).unwrap(),
            crate::tool_envelope::Status::Success
        );
        state
            .database()
            .unwrap()
            .save_setting(crate::capabilities::DISABLED_KEY, &vec!["local_execution"])
            .unwrap();
        let error = crate::harness::execute(
            &ctx,
            "check_run",
            json!({"command":"echo denied > denied.txt"}),
        )
        .await
        .err()
        .unwrap();
        assert!(error.contains("disabled"), "{error}");
        assert!(!root.path().join("denied.txt").exists());
        assert!(
            crate::harness::execute(&ctx, "check_report", json!({"id":id}))
                .await
                .is_err(),
            "A stale offered reader still honors capability kills"
        );
        state
            .database()
            .unwrap()
            .save_setting(crate::capabilities::DISABLED_KEY, &Vec::<String>::new())
            .unwrap();
        let unselected = crate::chat::assemble_turn(
            &state,
            &conversation,
            "Run checks",
            vec![],
            Some(vec![]),
            Some(crate::presets::STANDARD),
            None,
        )
        .await
        .unwrap();
        assert!(!unselected
            .tools
            .iter()
            .any(|tool| tool.alias == "check_run" || tool.alias == "check_report"));
        // Existing conversations retain their saved preset; the explicit
        // profile argument is intentionally only a new-chat default.
        let minimal = crate::chat::assemble_turn(
            &state,
            "",
            "Run checks",
            vec!["__execution".into()],
            Some(vec![]),
            Some(crate::presets::MINIMAL),
            None,
        )
        .await
        .unwrap();
        assert!(!minimal
            .tools
            .iter()
            .any(|tool| tool.alias == "check_run" || tool.alias == "check_report"));
        config.bash.enabled = false;
        state
            .database()
            .unwrap()
            .save_execution_config(&config)
            .unwrap();
        let unavailable = crate::chat::assemble_turn(
            &state,
            &conversation,
            "Read owned reports",
            vec!["__execution".into()],
            Some(vec![]),
            Some(crate::presets::STANDARD),
            None,
        )
        .await
        .unwrap();
        assert!(!unavailable
            .tools
            .iter()
            .any(|tool| tool.alias == "check_run"));
        assert!(
            unavailable
                .tools
                .iter()
                .any(|tool| tool.alias == "check_report"),
            "Owned handles remain readable when the shell is disabled"
        );
    }
}

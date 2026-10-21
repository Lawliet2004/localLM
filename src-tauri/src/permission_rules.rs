//! Rule-based permissions layered in front of the access mode.
//!
//! The access mode answers "how much may this session do at once". A rule
//! answers "what may this particular thing do, forever" — "always allow
//! `cargo test`", "never write outside `src/`". Rules are evaluated FIRST and
//! are authoritative: a matching `deny` refuses the call whatever the mode
//! says, a matching `ask` forces a prompt even under full access, and a
//! matching `allow` lets the call through whatever the mode says.
//!
//! Two invariants matter more than the matching details:
//!
//! 1. **A rule may only ever come from the user's persisted settings.**
//!    Nothing in this module writes a rule from model output, tool output or
//!    a tool argument. `save` is only reachable from the settings command.
//! 2. **An unreadable rule set is empty, never permissive.** A corrupted or
//!    missing entry yields no rules at all, so the access mode keeps deciding
//!    exactly as it did before rules existed.
//!
//! The wire format is the persisted JSON in [`RULES_KEY`]:
//!
//! ```json
//! {"rules":[{"action":"command","resource":"cargo test*","effect":"allow"},
//!           {"action":"path","resource":"src/**","effect":"allow"},
//!           {"action":"path","resource":"**/*.env","effect":"deny"}]}
//! ```

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::State;

/// The settings key the rule set is persisted under.
pub const RULES_KEY: &str = "permissions.rules";
/// At most this many rules may be persisted.
pub const MAX_RULES: usize = 256;
/// A rule resource is at most this many bytes long.
pub const MAX_RESOURCE_BYTES: usize = 1024;
/// The authorization recorded when a rule allows a call.
pub const RULE_ALLOW_REASON: &str = "allowed by a permission rule";

/// What a rule applies to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Action {
    /// Any tool by alias.
    Tool,
    /// A shell command by prefix, e.g. "cargo test".
    Command,
    /// A filesystem path, workspace-relative or absolute.
    Path,
    /// A web host by domain.
    Host,
}

impl Action {
    /// The wire name used in persisted rules and in the UI.
    pub fn as_str(&self) -> &'static str {
        match self {
            Action::Tool => "tool",
            Action::Command => "command",
            Action::Path => "path",
            Action::Host => "host",
        }
    }

    /// Parse a wire name. Case and surrounding spaces are ignored.
    pub fn parse(value: &str) -> Option<Action> {
        match value.trim().to_ascii_lowercase().as_str() {
            "tool" => Some(Action::Tool),
            "command" => Some(Action::Command),
            "path" => Some(Action::Path),
            "host" => Some(Action::Host),
            _ => None,
        }
    }
}

/// What happens when a rule matches.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Effect {
    Allow,
    Deny,
    Ask,
}

impl Effect {
    /// The wire name used in persisted rules and in the UI.
    pub fn as_str(&self) -> &'static str {
        match self {
            Effect::Allow => "allow",
            Effect::Deny => "deny",
            Effect::Ask => "ask",
        }
    }

    /// Parse a wire name. Case and surrounding spaces are ignored.
    pub fn parse(value: &str) -> Option<Effect> {
        match value.trim().to_ascii_lowercase().as_str() {
            "allow" => Some(Effect::Allow),
            "deny" => Some(Effect::Deny),
            "ask" => Some(Effect::Ask),
            _ => None,
        }
    }
}

/// One rule. Later rules win over earlier ones.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Rule {
    pub action: Action,
    pub resource: String,
    pub effect: Effect,
}

/// A persisted, ordered set of rules.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RuleSet {
    pub rules: Vec<Rule>,
}

/// The combined decision for one call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Decision {
    /// Proceed without asking.
    Allow(&'static str),
    /// Ask the user.
    Ask,
    /// Refuse, with the reason.
    Deny(String),
}

/// The rule shape the UI exchanges with the backend.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuleView {
    pub action: String,
    pub resource: String,
    pub effect: String,
}

impl RuleView {
    /// The view of a stored rule.
    pub fn from_rule(rule: &Rule) -> RuleView {
        RuleView {
            action: rule.action.as_str().to_string(),
            resource: rule.resource.clone(),
            effect: rule.effect.as_str().to_string(),
        }
    }
}

impl RuleSet {
    /// Whether any rule matches. The LAST matching rule decides.
    pub fn evaluate(&self, action: Action, resource: &str) -> Option<Effect> {
        let mut decided: Option<Effect> = None;
        for rule in &self.rules {
            if rule.action == action && resource_matches(action, &rule.resource, resource) {
                decided = Some(rule.effect);
            }
        }
        decided
    }
}

/// Read the persisted rule set. A missing or unparsable entry yields an
/// empty set — never a permissive default.
pub fn load(store: &crate::store::Store) -> RuleSet {
    store.setting::<RuleSet>(RULES_KEY).unwrap_or_default()
}

/// Persist the rule set.
pub fn save(store: &crate::store::Store, rules: &RuleSet) -> Result<(), String> {
    store.save_setting(RULES_KEY, rules)
}

/// Validate the views the UI sent and turn them into a rule set.
///
/// The error names the offending rule (1-based) so the UI can point at it.
pub fn validate_views(views: &[RuleView]) -> Result<RuleSet, String> {
    if views.len() > MAX_RULES {
        return Err(format!(
            "At most {MAX_RULES} permission rules can be saved."
        ));
    }
    let mut rules = Vec::with_capacity(views.len());
    for (index, view) in views.iter().enumerate() {
        let position = index + 1;
        let action = Action::parse(&view.action)
            .ok_or_else(|| format!("Rule {position}: unknown action '{}'.", view.action))?;
        let effect = Effect::parse(&view.effect)
            .ok_or_else(|| format!("Rule {position}: unknown effect '{}'.", view.effect))?;
        if view.resource.is_empty() || view.resource.len() > MAX_RESOURCE_BYTES {
            return Err(format!(
                "Rule {position}: the resource must be 1-{MAX_RESOURCE_BYTES} bytes."
            ));
        }
        rules.push(Rule {
            action,
            resource: view.resource.clone(),
            effect,
        });
    }
    Ok(RuleSet { rules })
}

/// Combine the persisted rules with the access mode.
///
/// Rules are evaluated FIRST and are authoritative:
/// - A matching `Deny` refuses the call regardless of access mode.
/// - A matching `Allow` permits the call regardless of access mode, but ONLY
///   when the rule is explicit. An `Allow` rule still does not override a
///   tool that requires explicit approval for its own reasons (a tool
///   flagged `x-locallm-always-ask`).
/// - A matching `Ask` forces a prompt even under Full Access.
/// - When no rule matches, fall through to the access mode exactly as today.
///
/// CRITICAL: a rule may never be created or changed by model output. Rules
/// come only from the user's persisted settings.
pub fn decide(
    rules: &RuleSet,
    mode: crate::permissions::AccessMode,
    action: Action,
    resource: &str,
    trusted_read: bool,
) -> Decision {
    match rules.evaluate(action, resource) {
        Some(Effect::Deny) => Decision::Deny(deny_reason(action, resource)),
        Some(Effect::Ask) => Decision::Ask,
        Some(Effect::Allow) => Decision::Allow(RULE_ALLOW_REASON),
        None => match mode.automatic_reason(trusted_read) {
            Some(reason) => Decision::Allow(reason),
            None => Decision::Ask,
        },
    }
}

/// [`decide`] plus the tool's own approval policy.
///
/// A tool that requires explicit approval for its own reasons (the
/// `x-locallm-always-ask` pin, or a shell policy that demands a prompt) keeps
/// that requirement: an `allow` rule narrows nothing away from it and the
/// call is still confirmed by the user.
pub fn decide_for_tool(
    rules: &RuleSet,
    mode: crate::permissions::AccessMode,
    action: Action,
    resource: &str,
    trusted_read: bool,
    tool_requires_approval: bool,
) -> Decision {
    if tool_requires_approval && matches!(rules.evaluate(action, resource), Some(Effect::Allow)) {
        return Decision::Ask;
    }
    decide(rules, mode, action, resource, trusted_read)
}

/// The rule action and resource that describe a tool call.
///
/// For a shell tool the resource is the command; for a workspace file tool
/// it is the path; for a web tool it is the host; otherwise it is the
/// tool alias under `Action::Tool`.
pub fn classify(alias: &str, args: &Value) -> (Action, String) {
    match alias {
        "shell_run" | "process_start" | "check_run" => {
            (Action::Command, string_arg(args, &["command"]))
        }
        "run_code" => (Action::Command, string_arg(args, &["code"])),
        // A terminal receives the shell line it is asked to run.
        "terminal_send" => (Action::Command, string_arg(args, &["command", "input"])),
        "workspace_edit_file"
        | "workspace_write_file"
        | "workspace_create_file"
        | "workspace_open_preview"
        | "file_write"
        | "file_read"
        | "file_search" => (Action::Path, string_arg(args, &["path"])),
        // A patch touches several files; the first changed path decides.
        "workspace_apply_patch" => (Action::Path, patch_path(args)),
        "web_fetch" | "web_fetch_url" | "web_read" => (Action::Host, host_arg(args)),
        // Everything else — web_search included — is addressed by its alias.
        _ => (Action::Tool, alias.to_string()),
    }
}

/// List the persisted rules.
#[tauri::command]
pub fn list_permission_rules(state: State<'_, crate::AppState>) -> Result<Vec<RuleView>, String> {
    let store = state.database()?;
    Ok(load(&store).rules.iter().map(RuleView::from_rule).collect())
}

/// Replace the persisted rules.
#[tauri::command]
pub fn save_permission_rules(
    state: State<'_, crate::AppState>,
    rules: Vec<RuleView>,
) -> Result<(), String> {
    let rule_set = validate_views(&rules)?;
    let store = state.database()?;
    save(&store, &rule_set)
}

fn deny_reason(action: Action, resource: &str) -> String {
    if resource.is_empty() {
        format!("Denied by a permission rule for {}.", action.as_str())
    } else {
        format!(
            "Denied by a permission rule for {} '{}'.",
            action.as_str(),
            resource
        )
    }
}

/// The first non-empty string argument among `keys`.
fn string_arg(args: &Value, keys: &[&str]) -> String {
    for key in keys {
        if let Some(value) = args.get(*key).and_then(Value::as_str) {
            if !value.is_empty() {
                return value.to_string();
            }
        }
    }
    String::new()
}

/// The first changed path of an `apply_patch` call.
fn patch_path(args: &Value) -> String {
    args.get("changes")
        .and_then(Value::as_array)
        .and_then(|changes| changes.first())
        .and_then(|change| change.get("path"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

/// The host of a `url` argument.
fn host_arg(args: &Value) -> String {
    let url = args.get("url").and_then(Value::as_str).unwrap_or_default();
    host_from_url(url)
}

/// The host of a URL: no scheme, no user info, no port, lower case.
pub fn host_from_url(url: &str) -> String {
    let without_scheme = match url.find("://") {
        Some(index) => &url[index + 3..],
        None => url,
    };
    let authority = without_scheme
        .split(['/', '?', '#'])
        .next()
        .unwrap_or_default();
    let authority = match authority.rsplit_once('@') {
        Some((_, host)) => host,
        None => authority,
    };
    let host = match authority.rsplit_once(':') {
        Some((host, port)) if port.chars().all(|character| character.is_ascii_digit()) => host,
        _ => authority,
    };
    host.trim().to_ascii_lowercase()
}

/// Whether a rule resource covers the resource a call actually touches.
fn resource_matches(action: Action, pattern: &str, resource: &str) -> bool {
    match action {
        Action::Tool => tool_matches(pattern, resource),
        Action::Command => command_matches(pattern, resource),
        Action::Path => path_matches(pattern, resource),
        Action::Host => host_matches(pattern, resource),
    }
}

/// A tool alias matches exactly, or by a trailing `*` wildcard.
fn tool_matches(pattern: &str, alias: &str) -> bool {
    if pattern == alias {
        return true;
    }
    match pattern.strip_suffix('*') {
        Some(prefix) => alias.starts_with(prefix),
        None => false,
    }
}

/// A command matches the whole command line.
///
/// `*` matches zero or more characters and `?` exactly one. A pattern that
/// ends with " *" also matches the bare command without the trailing space,
/// so "cargo test *" covers "cargo test" as well as "cargo test --lib".
fn command_matches(pattern: &str, command: &str) -> bool {
    if let Some(prefix) = pattern.strip_suffix(" *") {
        if command == prefix {
            return true;
        }
    }
    glob_matches(
        &tokenize(pattern, false),
        &command.chars().collect::<Vec<char>>(),
        None,
        false,
    )
}

/// A host matches the whole host name, case-insensitively.
///
/// `*` and `?` never cross a dot, so "*.example.com" covers "api.example.com"
/// but not "example.com", and "example.*" covers "example.org" but not
/// "example.evil.com".
fn host_matches(pattern: &str, host: &str) -> bool {
    let pattern = pattern.trim().to_ascii_lowercase();
    let host = host.trim().to_ascii_lowercase();
    glob_matches(
        &tokenize(&pattern, false),
        &host.chars().collect::<Vec<char>>(),
        Some('.'),
        false,
    )
}

/// A path matches the whole path.
///
/// `**` crosses directory separators, `*` and `?` do not. A relative pattern
/// also matches a path that ends with it, so "src/main.rs" covers
/// "project/src/main.rs". Backslashes are normalized to forward slashes
/// before matching.
fn path_matches(pattern: &str, path: &str) -> bool {
    let pattern = normalize_separators(pattern);
    let path = normalize_separators(path);
    if pattern.is_empty() {
        return path.is_empty();
    }
    // A rooted pattern is anchored at the filesystem root (or at the home
    // directory when it starts with "~").
    if pattern.starts_with('/') {
        return path_glob_matches(&pattern, &path);
    }
    if pattern.starts_with('~') {
        if path_glob_matches(&pattern, &path) {
            return true;
        }
        if let Some(home) = home_directory() {
            let expanded = format!(
                "{home}/{}",
                pattern.trim_start_matches('~').trim_start_matches('/')
            );
            if path_glob_matches(&expanded, &path) {
                return true;
            }
        }
        return false;
    }
    // A pattern that starts with a glob is anchored at the start of the path.
    if pattern.starts_with('*') || pattern.starts_with('?') {
        if path_glob_matches(&pattern, &path) {
            return true;
        }
        // "**/name" also covers a file that sits directly in the root.
        if let Some(rest) = pattern.strip_prefix("**/") {
            return path_glob_matches(rest, &path);
        }
        return false;
    }
    // A relative pattern matches the path itself or any path that ends with
    // it on a directory boundary.
    if path_glob_matches(&pattern, &path) {
        return true;
    }
    let mut start = 0;
    while let Some(offset) = path[start..].find('/') {
        start += offset + 1;
        if path_glob_matches(&pattern, &path[start..]) {
            return true;
        }
    }
    false
}

/// The path glob with its path-specific extras.
fn path_glob_matches(pattern: &str, path: &str) -> bool {
    let characters = path.chars().collect::<Vec<char>>();
    if glob_matches(&tokenize(pattern, true), &characters, Some('/'), true) {
        return true;
    }
    // "src/**" also covers the directory itself.
    if let Some(prefix) = pattern.strip_suffix("/**") {
        return glob_matches(&tokenize(prefix, true), &characters, Some('/'), true);
    }
    false
}

/// The home directory of the current user, when the platform reports one.
fn home_directory() -> Option<String> {
    std::env::var("HOME")
        .ok()
        .or_else(|| std::env::var("USERPROFILE").ok())
        .filter(|home| !home.trim().is_empty())
}

/// Backslashes are directory separators on Windows and nothing else.
fn normalize_separators(value: &str) -> String {
    value.replace('\\', "/")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Token {
    Literal(char),
    Any,
    Star,
    DoubleStar,
}

/// Split a pattern into its tokens.
///
/// `double_star` decides whether "**" is one separator-crossing token or two
/// ordinary stars.
fn tokenize(pattern: &str, double_star: bool) -> Vec<Token> {
    let characters = pattern.chars().collect::<Vec<char>>();
    let mut tokens = Vec::with_capacity(characters.len());
    let mut index = 0;
    while index < characters.len() {
        match characters[index] {
            '*' => {
                if double_star && index + 1 < characters.len() && characters[index + 1] == '*' {
                    tokens.push(Token::DoubleStar);
                    index += 2;
                } else {
                    tokens.push(Token::Star);
                    index += 1;
                }
            }
            '?' => {
                tokens.push(Token::Any);
                index += 1;
            }
            other => {
                tokens.push(Token::Literal(other));
                index += 1;
            }
        }
    }
    tokens
}

/// Match a tokenized pattern against a value, whole value.
///
/// `separator` is the character a single `*` or `?` may not cross; `None`
/// lets them cross everything. `double_star_crosses` lets a `**` token cross
/// the separator as well.
fn glob_matches(
    tokens: &[Token],
    value: &[char],
    separator: Option<char>,
    double_star_crosses: bool,
) -> bool {
    let mut pattern_index = 0usize;
    let mut value_index = 0usize;
    // Every star that is still open, oldest first. A mismatch widens the
    // newest star; when it cannot widen any further the older star takes
    // over, which is what lets "**/*.env" match "a/b/.env".
    let mut stars: Vec<(usize, usize)> = Vec::new();
    loop {
        if value_index < value.len() {
            if pattern_index < tokens.len() {
                match tokens[pattern_index] {
                    Token::Literal(expected) if value[value_index] == expected => {
                        pattern_index += 1;
                        value_index += 1;
                        continue;
                    }
                    Token::Any
                        if separator.is_none_or(|barrier| value[value_index] != barrier) =>
                    {
                        pattern_index += 1;
                        value_index += 1;
                        continue;
                    }
                    Token::Star | Token::DoubleStar => {
                        stars.push((pattern_index, value_index));
                        pattern_index += 1;
                        continue;
                    }
                    _ => {}
                }
            }
            match widen_star(&mut stars, tokens, value, separator, double_star_crosses) {
                Some((next_pattern, next_value)) => {
                    pattern_index = next_pattern;
                    value_index = next_value;
                }
                None => return false,
            }
            continue;
        }
        // The value is exhausted: only stars may remain.
        if tokens[pattern_index..]
            .iter()
            .all(|token| matches!(token, Token::Star | Token::DoubleStar))
        {
            return true;
        }
        match widen_star(&mut stars, tokens, value, separator, double_star_crosses) {
            Some((next_pattern, next_value)) => {
                pattern_index = next_pattern;
                value_index = next_value;
            }
            None => return false,
        }
    }
}

/// Widen the newest open star by one character, or fall back to an older
/// one when it cannot widen any further.
fn widen_star(
    stars: &mut Vec<(usize, usize)>,
    tokens: &[Token],
    value: &[char],
    separator: Option<char>,
    double_star_crosses: bool,
) -> Option<(usize, usize)> {
    while let Some((star_index, star_value)) = stars.pop() {
        if star_value >= value.len() {
            continue;
        }
        let crosses = separator.is_none()
            || (double_star_crosses && matches!(tokens[star_index], Token::DoubleStar));
        if crosses || Some(value[star_value]) != separator {
            let next = star_value + 1;
            stars.push((star_index, next));
            return Some((star_index + 1, next));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn rule(action: Action, resource: &str, effect: Effect) -> Rule {
        Rule {
            action,
            resource: resource.to_string(),
            effect,
        }
    }

    fn rules(rules: Vec<Rule>) -> RuleSet {
        RuleSet { rules }
    }

    #[test]
    fn action_and_effect_use_stable_wire_names() {
        assert_eq!(Action::Tool.as_str(), "tool");
        assert_eq!(Action::Command.as_str(), "command");
        assert_eq!(Action::Path.as_str(), "path");
        assert_eq!(Action::Host.as_str(), "host");
        assert_eq!(Effect::Allow.as_str(), "allow");
        assert_eq!(Effect::Deny.as_str(), "deny");
        assert_eq!(Effect::Ask.as_str(), "ask");
        for action in [Action::Tool, Action::Command, Action::Path, Action::Host] {
            assert_eq!(Action::parse(action.as_str()), Some(action));
        }
        for effect in [Effect::Allow, Effect::Deny, Effect::Ask] {
            assert_eq!(Effect::parse(effect.as_str()), Some(effect));
        }
        assert_eq!(Action::parse("nonsense"), None);
        assert_eq!(Effect::parse("nonsense"), None);
    }

    #[test]
    fn wildcards_match_zero_or_more_and_exactly_one_character() {
        assert!(command_matches("*", ""));
        assert!(command_matches("*", "cargo test"));
        assert!(command_matches("cargo*", "cargo test"));
        assert!(command_matches("*test", "cargo test"));
        // "?" covers exactly one character, the space included.
        assert!(command_matches("cargo?test", "cargo test"));
        assert!(!command_matches("cargo?test", "cargo  test"));
        assert!(!command_matches("cargo?test", "cargo test "));
        assert!(!command_matches("cargo?test", "cargotest"));
    }

    #[test]
    fn matching_covers_the_whole_value() {
        assert!(!command_matches("car", "cargo test"));
        assert!(command_matches("car", "car"));
        assert!(!command_matches("cargo test", "cargo test --lib"));
    }

    #[test]
    fn a_trailing_star_also_matches_the_bare_value() {
        assert!(command_matches("cargo test *", "cargo test"));
        assert!(command_matches("cargo test *", "cargo test --lib"));
        assert!(!command_matches("cargo test *", "cargo build"));
    }

    #[test]
    fn the_last_matching_rule_wins() {
        let allow_then_deny = rules(vec![
            rule(Action::Command, "cargo test*", Effect::Allow),
            rule(Action::Command, "cargo test --lib*", Effect::Deny),
        ]);
        assert_eq!(
            allow_then_deny.evaluate(Action::Command, "cargo test --lib"),
            Some(Effect::Deny)
        );
        assert_eq!(
            allow_then_deny.evaluate(Action::Command, "cargo test"),
            Some(Effect::Allow)
        );
        let deny_then_allow = rules(vec![
            rule(Action::Command, "cargo test --lib*", Effect::Deny),
            rule(Action::Command, "cargo test*", Effect::Allow),
        ]);
        assert_eq!(
            deny_then_allow.evaluate(Action::Command, "cargo test --lib"),
            Some(Effect::Allow)
        );
    }

    #[test]
    fn a_rule_set_without_a_match_decides_nothing() {
        let empty = RuleSet::default();
        assert_eq!(empty.evaluate(Action::Command, "cargo test"), None);
        let other = rules(vec![rule(Action::Command, "cargo test*", Effect::Allow)]);
        assert_eq!(other.evaluate(Action::Path, "cargo test"), None);
        assert_eq!(other.evaluate(Action::Command, "npm run dev"), None);
    }

    #[test]
    fn path_patterns_follow_directory_boundaries() {
        assert!(path_matches("src/**", "src/a/b.rs"));
        assert!(path_matches("src/**", "src"));
        assert!(!path_matches("src/**", "other/a.rs"));
        assert!(path_matches("**/*.env", "a/b/.env"));
        assert!(path_matches("**/*.env", ".env"));
        assert!(path_matches("src/main.rs", "src/main.rs"));
        assert!(path_matches("src/main.rs", "project/src/main.rs"));
        assert!(!path_matches("src/main.rs", "src/other.rs"));
        assert!(path_matches("src\\main.rs", "src/main.rs"));
    }

    #[test]
    fn host_patterns_match_whole_labels_only() {
        assert!(host_matches("*.example.com", "api.example.com"));
        assert!(!host_matches("*.example.com", "example.com"));
        assert!(host_matches("example.*", "example.org"));
        assert!(!host_matches("example.*", "example.evil.com"));
        assert!(host_matches("*.EXAMPLE.com", "API.example.COM"));
        assert!(host_matches("example.com", "example.com"));
        assert!(!host_matches("example.com", "api.example.com"));
    }

    #[test]
    fn tool_patterns_match_the_alias_or_a_trailing_wildcard() {
        assert!(tool_matches("shell_run", "shell_run"));
        assert!(!tool_matches("shell_run", "shell_run_all"));
        assert!(tool_matches("shell*", "shell_run"));
        assert!(!tool_matches("shell*", "process_start"));
    }

    #[test]
    fn a_deny_rule_beats_full_access() {
        let set = rules(vec![rule(Action::Command, "rm *", Effect::Deny)]);
        let decision = decide(
            &set,
            crate::permissions::AccessMode::FullAccess,
            Action::Command,
            "rm -rf build",
            false,
        );
        assert!(matches!(decision, Decision::Deny(reason) if reason.contains("rm -rf build")));
    }

    #[test]
    fn an_ask_rule_forces_a_prompt_under_full_access() {
        let set = rules(vec![rule(Action::Command, "cargo *", Effect::Ask)]);
        let decision = decide(
            &set,
            crate::permissions::AccessMode::FullAccess,
            Action::Command,
            "cargo test",
            false,
        );
        assert_eq!(decision, Decision::Ask);
    }

    #[test]
    fn an_allow_rule_permits_a_call_under_ask_mode() {
        let set = rules(vec![rule(Action::Command, "cargo test*", Effect::Allow)]);
        let decision = decide(
            &set,
            crate::permissions::AccessMode::Ask,
            Action::Command,
            "cargo test",
            false,
        );
        assert_eq!(decision, Decision::Allow(RULE_ALLOW_REASON));
    }

    #[test]
    fn without_a_rule_the_access_mode_decides_exactly_as_before() {
        let empty = RuleSet::default();
        for mode in [
            crate::permissions::AccessMode::Ask,
            crate::permissions::AccessMode::AutoApprove,
            crate::permissions::AccessMode::FullAccess,
        ] {
            for trusted_read in [true, false] {
                let decision = decide(&empty, mode, Action::Command, "cargo test", trusted_read);
                match mode.automatic_reason(trusted_read) {
                    Some(reason) => assert_eq!(decision, Decision::Allow(reason)),
                    None => assert_eq!(decision, Decision::Ask),
                }
            }
        }
        // A trusted read still auto-approves under auto-approve.
        let decision = decide(
            &empty,
            crate::permissions::AccessMode::AutoApprove,
            Action::Path,
            "src/main.rs",
            true,
        );
        assert!(matches!(decision, Decision::Allow(_)));
    }

    #[test]
    fn an_allow_rule_never_overrides_a_tool_that_always_asks() {
        let set = rules(vec![rule(Action::Tool, "git_commit*", Effect::Allow)]);
        let decision = decide_for_tool(
            &set,
            crate::permissions::AccessMode::FullAccess,
            Action::Tool,
            "git_commit",
            false,
            true,
        );
        assert_eq!(decision, Decision::Ask);
        let without_pin = decide_for_tool(
            &set,
            crate::permissions::AccessMode::FullAccess,
            Action::Tool,
            "git_commit",
            false,
            false,
        );
        assert_eq!(without_pin, Decision::Allow(RULE_ALLOW_REASON));
    }

    #[test]
    fn a_rule_set_that_fails_to_parse_loads_as_empty() {
        let store = crate::store::Store::open_memory().unwrap();
        store.save_setting(RULES_KEY, &"not a rule set").unwrap();
        assert!(load(&store).rules.is_empty());
        // A saved set round-trips.
        let set = rules(vec![rule(Action::Command, "cargo test*", Effect::Allow)]);
        save(&store, &set).unwrap();
        let loaded = load(&store);
        assert_eq!(loaded.rules.len(), 1);
        assert_eq!(
            loaded.evaluate(Action::Command, "cargo test"),
            Some(Effect::Allow)
        );
    }

    #[test]
    fn saved_rules_use_the_documented_wire_format() {
        let set = rules(vec![
            rule(Action::Command, "cargo test*", Effect::Allow),
            rule(Action::Path, "**/*.env", Effect::Deny),
        ]);
        let json = serde_json::to_string(&set).unwrap();
        assert_eq!(
            json,
            r#"{"rules":[{"action":"command","resource":"cargo test*","effect":"allow"},{"action":"path","resource":"**/*.env","effect":"deny"}]}"#
        );
        let parsed: RuleSet = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, set);
    }

    #[test]
    fn invalid_rules_are_rejected_with_their_position() {
        let views = vec![RuleView {
            action: "command".into(),
            resource: "cargo test".into(),
            effect: "maybe".into(),
        }];
        let error = validate_views(&views).unwrap_err();
        assert!(error.contains("Rule 1"), "{error}");
        let empty_resource = vec![RuleView {
            action: "command".into(),
            resource: String::new(),
            effect: "allow".into(),
        }];
        assert!(validate_views(&empty_resource).is_err());
        let unknown_action = vec![RuleView {
            action: "shell".into(),
            resource: "cargo test".into(),
            effect: "allow".into(),
        }];
        assert!(validate_views(&unknown_action)
            .unwrap_err()
            .contains("Rule 1"));
        let too_many: Vec<RuleView> = (0..=MAX_RULES)
            .map(|_| RuleView {
                action: "command".into(),
                resource: "cargo test".into(),
                effect: "allow".into(),
            })
            .collect();
        assert!(validate_views(&too_many).is_err());
    }

    #[test]
    fn classify_maps_the_documented_aliases() {
        assert_eq!(
            classify("shell_run", &json!({"command": "cargo test"})),
            (Action::Command, "cargo test".to_string())
        );
        assert_eq!(
            classify("process_start", &json!({"command": "npm run dev"})),
            (Action::Command, "npm run dev".to_string())
        );
        assert_eq!(
            classify("check_run", &json!({"command": "cargo check"})),
            (Action::Command, "cargo check".to_string())
        );
        assert_eq!(
            classify("run_code", &json!({"code": "print(1)"})),
            (Action::Command, "print(1)".to_string())
        );
        assert_eq!(
            classify("terminal_send", &json!({"id": "t1", "input": "ls -la"})),
            (Action::Command, "ls -la".to_string())
        );
        for alias in [
            "workspace_edit_file",
            "workspace_write_file",
            "workspace_create_file",
            "file_write",
            "file_read",
            "file_search",
        ] {
            assert_eq!(
                classify(alias, &json!({"path": "src/main.rs"})),
                (Action::Path, "src/main.rs".to_string()),
                "{alias}"
            );
        }
        assert_eq!(
            classify(
                "workspace_apply_patch",
                &json!({"changes": [{"kind": "edit", "path": "a.txt"}, {"kind": "edit", "path": "b.txt"}]})
            ),
            (Action::Path, "a.txt".to_string())
        );
        assert_eq!(
            classify("web_fetch", &json!({"url": "https://Example.com/a/b"})),
            (Action::Host, "example.com".to_string())
        );
        assert_eq!(
            classify(
                "web_fetch_url",
                &json!({"url": "http://example.org:8080/x"})
            ),
            (Action::Host, "example.org".to_string())
        );
        assert_eq!(
            classify("web_read", &json!({"url": "https://example.com"})),
            (Action::Host, "example.com".to_string())
        );
        assert_eq!(
            classify("web_search", &json!({"question": "why"})),
            (Action::Tool, "web_search".to_string())
        );
        assert_eq!(
            classify("todo_write", &json!({})),
            (Action::Tool, "todo_write".to_string())
        );
    }
}

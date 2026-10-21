//! Typed, user-reviewed MCP input handling.
//!
//! Two protocol surfaces share one channel:
//! - legacy `elicitation/create` requests ([`McpSessionHandler`])
//! - bounded MRTR `inputRequired` continuations on tools/call,
//!   resources/read and prompts/get ([`InputScope::fulfill`])
//!
//! A pending question owns its full original schema (bounded to the supported
//! form subset) plus a oneshot answer channel. Each question is bound to one
//! associated call on one connector by a per-session gate and a monotonically
//! increasing token, so a question can never outlive its operation and the
//! id-scoped close event cannot dismiss a newer request. URL-mode elicitation,
//! sampling and roots remain unadvertised and unsupported; a `task` response
//! produces an explicit unsupported-task error rather than a silent job.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use rmcp::handler::client::ClientHandler;
use rmcp::model::{
    ClientInfo, ElicitRequestParams, ElicitResult, ElicitationAction, ElicitationCapability,
    ElicitationSchema, EnumSchema, FormElicitationCapability, InputRequest, InputRequests,
    InputResponses, MultiSelectEnumSchema, PrimitiveSchemaDefinition, SingleSelectEnumSchema,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData, RoleClient};
use serde_json::{json, Map, Value};
use tokio::sync::OwnedSemaphorePermit;

/// Continuation rounds allowed for one call before it is abandoned.
pub const MAX_CONTINUATION_ROUNDS: usize = 4;
/// Server-asked questions allowed within one call across all rounds.
const MAX_QUESTIONS_PER_CALL: usize = 8;
/// Input requests accepted per continuation round.
const MAX_QUESTIONS_PER_ROUND: usize = 4;
/// Opaque request_state is echoed verbatim but bounded before re-sending.
pub const MAX_REQUEST_STATE_BYTES: usize = 64 * 1024;
/// Server-provided input-request map key bound (keys are preserved exactly).
const MAX_REQUEST_KEY_BYTES: usize = 256;
/// Supported form fields per question.
const MAX_FORM_FIELDS: usize = 32;
/// Options allowed on a select or multi-select field.
const MAX_OPTIONS: usize = 64;
/// Server message and label bounds shown to the user as text.
const MAX_MESSAGE_BYTES: usize = 4096;
const MAX_LABEL_BYTES: usize = 1024;
/// Total accepted answer payload.
const MAX_ANSWER_BYTES: usize = 64 * 1024;
/// Global pending-question bound across every connector session.
const MAX_PENDING: usize = 64;
/// How long a form may await a decision before it cancels itself.
const FORM_WAIT: Duration = Duration::from_secs(600);
/// How long a call may wait for the per-session input gate.
const GATE_WAIT: Duration = Duration::from_secs(120);

/// Process-wide scope sequence.
///
/// A scope id identifies one associated call. The pending-question registry
/// is global, so scope ids must be unique across every connector session: a
/// per-session counter would hand two connectors the same id for their first
/// call, and one connector's cleanup would then cancel the other connector's
/// pending form.
static SCOPE_SEQUENCE: AtomicU64 = AtomicU64::new(1);

/// Mint a scope id that is unique for the whole process.
fn mint_scope() -> u64 {
    SCOPE_SEQUENCE.fetch_add(1, Ordering::SeqCst)
}

fn lock<'a, T>(mutex: &'a Mutex<T>) -> MutexGuard<'a, T> {
    mutex.lock().unwrap_or_else(|error| error.into_inner())
}

/// Host-emitted event payload for one pending or closing input request.
fn approval_event(
    id: &str,
    connector: &str,
    local_name: Option<&str>,
    operation: &str,
    form: Option<Value>,
) -> Value {
    json!({"approval": {
        "id": id,
        "kind": "mcp_input",
        "connector": connector,
        "localServerName": local_name,
        "name": operation,
        "arguments": {},
        "form": form,
        "closed": form.is_none(),
    }})
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum InputAction {
    Accept,
    Decline,
    Cancel,
}

impl InputAction {
    fn parse(action: &str) -> Result<Self, String> {
        match action {
            "accept" => Ok(Self::Accept),
            "decline" => Ok(Self::Decline),
            "cancel" => Ok(Self::Cancel),
            _ => Err("MCP input action must be 'accept', 'decline' or 'cancel'.".into()),
        }
    }
    fn sdk(self) -> ElicitationAction {
        match self {
            Self::Accept => ElicitationAction::Accept,
            Self::Decline => ElicitationAction::Decline,
            Self::Cancel => ElicitationAction::Cancel,
        }
    }
}

pub struct Answer {
    pub action: InputAction,
    pub content: Option<Value>,
}

struct PendingInput {
    scope: u64,
    /// The exact serialized [`ElicitationSchema`] the answer must satisfy.
    schema: Value,
    sender: tokio::sync::oneshot::Sender<Answer>,
}

/// Typed request/response registry shared by every connector session. Model
/// tools can never resolve entries: only the dedicated UI command calls
/// [`Registry::resolve`], and entries die with their associated call scope.
#[derive(Default)]
pub struct Registry {
    pending: Mutex<HashMap<String, PendingInput>>,
}

impl Registry {
    fn begin(
        &self,
        scope: u64,
        schema: Value,
    ) -> Result<(String, tokio::sync::oneshot::Receiver<Answer>), String> {
        let (sender, receiver) = tokio::sync::oneshot::channel();
        let mut pending = lock(&self.pending);
        if pending.len() >= MAX_PENDING {
            return Err("Too many MCP input requests are already waiting.".into());
        }
        let id = uuid::Uuid::new_v4().to_string();
        pending.insert(
            id.clone(),
            PendingInput {
                scope,
                schema,
                sender,
            },
        );
        Ok((id, receiver))
    }

    /// Resolve a pending request. Invalid answers are rejected while the entry
    /// stays pending so the user can correct them. Decline/cancel discard any
    /// supplied content — only `accept` may carry a payload.
    pub fn resolve(&self, id: &str, action: &str, content: Option<Value>) -> Result<(), String> {
        let action = InputAction::parse(action)?;
        let mut pending = lock(&self.pending);
        let Some(entry) = pending.get(id) else {
            return Err("This input request already closed or expired.".into());
        };
        let content = match action {
            InputAction::Accept => {
                let content = content.ok_or("Accepting requires the requested form content.")?;
                if !content.is_object() || content.to_string().len() > MAX_ANSWER_BYTES {
                    return Err(
                        "Accepted content must be a JSON object within the 64 KiB limit.".into(),
                    );
                }
                let declared = match entry.schema["properties"].as_object() {
                    Some(properties) => properties.clone(),
                    None => Map::new(),
                };
                for key in content.as_object().expect("checked object").keys() {
                    if !declared.contains_key(key) {
                        return Err(format!("The answer includes '{key}', which the requested schema does not declare."));
                    }
                }
                crate::tool_validation::validate(&entry.schema, &content)?;
                Some(content)
            }
            _ => None,
        };
        let entry = pending.remove(id).expect("entry checked above");
        let _ = entry.sender.send(Answer { action, content });
        Ok(())
    }

    /// Drop every pending entry belonging to a call scope; dropping each
    /// sender wakes the waiter with a closed-channel error.
    fn remove_scope(&self, scope: u64) -> Vec<String> {
        let mut pending = lock(&self.pending);
        let ids: Vec<String> = pending
            .iter()
            .filter(|(_, entry)| entry.scope == scope)
            .map(|(id, _)| id.clone())
            .collect();
        for id in &ids {
            pending.remove(id);
        }
        ids
    }

    fn remove(&self, id: &str) {
        lock(&self.pending).remove(id);
    }
}

/// Live user authority for one associated call: the shared registry plus the
/// chat-stream emitter that carries the form payload to the approval dialog.
#[derive(Clone)]
pub struct CallUi {
    pub registry: Arc<Registry>,
    pub emit: Arc<dyn Fn(String, Option<Value>) + Send + Sync>,
}

impl CallUi {
    pub fn new(
        registry: Arc<Registry>,
        emit: Arc<dyn Fn(String, Option<Value>) + Send + Sync>,
    ) -> Self {
        Self { registry, emit }
    }
}

struct ActiveUi {
    emit: Arc<dyn Fn(String, Option<Value>) + Send + Sync>,
    registry: Arc<Registry>,
}

struct ActiveInput {
    token: u64,
    operation: String,
    ui: Option<ActiveUi>,
    questions: usize,
    decision: Option<InputAction>,
}

/// Per-connector input state. One gate serializes interactive calls so at most
/// one question — and one dialog — is outstanding per session; `active`
/// records which associated call currently owns the channel.
pub struct SessionInput {
    gate: Arc<tokio::sync::Semaphore>,
    active: Mutex<Option<ActiveInput>>,
    pub source: String,
    pub local_name: Option<String>,
}

impl SessionInput {
    pub fn new(source: String, local_name: Option<String>) -> Arc<Self> {
        Arc::new(Self {
            gate: Arc::new(tokio::sync::Semaphore::new(1)),
            active: Mutex::new(None),
            source,
            local_name,
        })
    }

    /// Serialize this call on the session and register it as the associated
    /// operation allowed to ask questions. `ui == None` means the call may
    /// proceed but has no live user authority (detached/background contexts).
    pub async fn enter(
        self: &Arc<Self>,
        ui: Option<CallUi>,
        operation: &str,
    ) -> Result<InputScope, String> {
        let permit = tokio::time::timeout(GATE_WAIT, self.gate.clone().acquire_owned())
            .await
            .map_err(|_| {
                "Another operation on this connector is still waiting for input.".to_string()
            })?
            .map_err(|_| "The connector input gate is closed.".to_string())?;
        let token = mint_scope();
        *lock(&self.active) = Some(ActiveInput {
            token,
            operation: operation.chars().take(256).collect(),
            ui: ui.map(|ui| ActiveUi {
                emit: ui.emit,
                registry: ui.registry,
            }),
            questions: 0,
            decision: None,
        });
        Ok(InputScope {
            session: self.clone(),
            token,
            _permit: permit,
        })
    }

    fn leave(&self, token: u64) {
        let ui = {
            let mut active = lock(&self.active);
            if active.as_ref().is_some_and(|call| call.token == token) {
                active.take().and_then(|call| call.ui)
            } else {
                None
            }
        };
        if let Some(ui) = ui {
            for id in ui.registry.remove_scope(token) {
                (ui.emit)(
                    "MCP input closed".into(),
                    Some(approval_event(
                        &id,
                        &self.source,
                        self.local_name.as_deref(),
                        "",
                        None,
                    )),
                );
            }
        }
    }

    fn active_token(&self) -> Option<u64> {
        lock(&self.active).as_ref().map(|call| call.token)
    }

    fn decision(&self, token: u64) -> Option<InputAction> {
        lock(&self.active)
            .as_ref()
            .filter(|call| call.token == token)
            .and_then(|call| call.decision)
    }

    fn note_decision(&self, token: u64, action: InputAction) {
        let mut active = lock(&self.active);
        if let Some(call) = active
            .as_mut()
            .filter(|call| call.token == token && call.decision.is_none())
        {
            call.decision = Some(action);
        }
    }

    /// Ask one bounded form question on behalf of the associated call `token`.
    /// Decline and cancel are sticky for the remainder of the call: repeat
    /// questions answer themselves with the recorded action instead of
    /// re-presenting the same decision to the user.
    async fn ask(
        &self,
        token: u64,
        message: &str,
        schema: &ElicitationSchema,
    ) -> Result<ElicitResult, String> {
        if let Some(action) = self.decision(token) {
            return Ok(ElicitResult::new(action.sdk()));
        }
        let spec = form_spec(message, schema)?;
        let schema_json = serde_json::to_value(schema)
            .map_err(|error| format!("The server input schema could not be encoded: {error}"))?;
        let (emit, registry) = {
            let mut active = lock(&self.active);
            let Some(call) = active.as_mut().filter(|call| call.token == token) else {
                return Err(
                    "The associated operation already ended; this input request is unsolicited."
                        .into(),
                );
            };
            if call.questions >= MAX_QUESTIONS_PER_CALL {
                return Err(format!(
                    "The server asked more than {MAX_QUESTIONS_PER_CALL} questions in one call."
                ));
            }
            call.questions += 1;
            let Some(ui) = call.ui.as_ref() else {
                return Err(
                    "This operation requires interactive input that is unavailable here.".into(),
                );
            };
            (ui.emit.clone(), ui.registry.clone())
        };
        let (id, receiver) = registry.begin(token, schema_json)?;
        emit(
            format!("MCP input requested by {}", self.source),
            Some(approval_event(
                &id,
                &self.source,
                self.local_name.as_deref(),
                &self.operation_label(token),
                Some(spec),
            )),
        );
        let guard = PendingAsk {
            registry: &registry,
            emit: emit.as_ref(),
            id: &id,
            source: &self.source,
            local_name: self.local_name.as_deref(),
        };
        let answer = match tokio::time::timeout(FORM_WAIT, receiver).await {
            Ok(Ok(answer)) => answer,
            Ok(Err(_)) => {
                return Err("The input request was closed before a decision arrived.".into())
            }
            Err(_) => {
                self.note_decision(token, InputAction::Cancel);
                drop(guard);
                return Ok(ElicitResult::new(ElicitationAction::Cancel));
            }
        };
        drop(guard);
        if answer.action != InputAction::Accept {
            self.note_decision(token, answer.action);
        }
        let mut result = ElicitResult::new(answer.action.sdk());
        if let Some(content) = answer.content {
            result = result.with_content(content);
        }
        Ok(result)
    }

    fn operation_label(&self, token: u64) -> String {
        lock(&self.active)
            .as_ref()
            .filter(|call| call.token == token)
            .map(|call| call.operation.clone())
            .unwrap_or_default()
    }
}

/// Emits the id-scoped close event and drops the registry entry on any exit
/// path — resolution, timeout, cancellation or a dropped future — so the
/// dialog can never outlive its pending request.
struct PendingAsk<'a> {
    registry: &'a Registry,
    emit: &'a (dyn Fn(String, Option<Value>) + Send + Sync),
    id: &'a str,
    source: &'a str,
    local_name: Option<&'a str>,
}

impl Drop for PendingAsk<'_> {
    fn drop(&mut self) {
        self.registry.remove(self.id);
        (self.emit)(
            "MCP input closed".into(),
            Some(approval_event(
                self.id,
                self.source,
                self.local_name,
                "",
                None,
            )),
        );
    }
}

/// RAII handle for one associated call. Dropping it releases the session gate
/// and clears every pending question the call left behind.
pub struct InputScope {
    session: Arc<SessionInput>,
    token: u64,
    _permit: OwnedSemaphorePermit,
}

impl InputScope {
    /// The call may present a form to a live user.
    pub fn live(&self) -> bool {
        lock(&self.session.active)
            .as_ref()
            .is_some_and(|call| call.token == self.token && call.ui.is_some())
    }

    /// Whether the user already cancelled during this call; a `Complete`
    /// result racing a cancellation must not be reported as success.
    pub fn cancelled(&self) -> bool {
        self.session.decision(self.token) == Some(InputAction::Cancel)
    }

    pub async fn ask(
        &self,
        message: &str,
        schema: &ElicitationSchema,
    ) -> Result<ElicitResult, String> {
        self.session.ask(self.token, message, schema).await
    }

    /// Answer one MRTR input-request map. Exact server keys are preserved in
    /// `InputResponses`; unsupported request kinds fail explicitly.
    pub async fn fulfill(&self, requests: Option<InputRequests>) -> Result<InputResponses, String> {
        let requests = requests.unwrap_or_default();
        if requests.len() > MAX_QUESTIONS_PER_ROUND {
            return Err(format!(
                "The server asked more than {MAX_QUESTIONS_PER_ROUND} questions in one round."
            ));
        }
        let mut responses: InputResponses = BTreeMap::new();
        for (key, request) in requests {
            if key.len() > MAX_REQUEST_KEY_BYTES {
                return Err("The server sent an oversized input-request key.".into());
            }
            let result = match request {
                InputRequest::Elicitation(elicit) => self.answer_elicitation(elicit.params).await?,
                InputRequest::CreateMessage(_) => {
                    return Err(
                        "The server requested model sampling, which this client does not support."
                            .into(),
                    )
                }
                InputRequest::ListRoots(_) => return Err(
                    "The server requested workspace roots, which this client does not disclose."
                        .into(),
                ),
                _ => return Err("The server sent an unsupported input request type.".into()),
            };
            let value = serde_json::to_value(&result)
                .map_err(|error| format!("The input answer could not be encoded: {error}"))?;
            responses.insert(key, value);
        }
        Ok(responses)
    }

    async fn answer_elicitation(
        &self,
        params: ElicitRequestParams,
    ) -> Result<ElicitResult, String> {
        match params {
            ElicitRequestParams::FormElicitationParams {
                message,
                requested_schema,
                ..
            } => {
                if !self.live() {
                    return Err(
                        "This operation requires interactive input that is unavailable here."
                            .into(),
                    );
                }
                self.ask(&message, &requested_schema).await
            }
            ElicitRequestParams::UrlElicitationParams { .. } => Err(
                "The server requested URL-mode input, which is unsupported; no page was opened."
                    .into(),
            ),
            _ => Err("The server requested an unsupported elicitation mode.".into()),
        }
    }
}

impl Drop for InputScope {
    fn drop(&mut self) {
        self.session.leave(self.token);
    }
}

/// Client handler for one connected session. Advertises form elicitation only;
/// sampling, roots and URL mode stay unadvertised and fall through to the SDK
/// defaults (method-not-found / empty), which disclose nothing.
pub struct McpSessionHandler {
    session: Arc<SessionInput>,
}

impl McpSessionHandler {
    pub fn new(session: Arc<SessionInput>) -> Self {
        Self { session }
    }
}

impl ClientHandler for McpSessionHandler {
    fn get_info(&self) -> ClientInfo {
        let mut info = ClientInfo::default();
        info.capabilities.elicitation = Some(
            ElicitationCapability::new()
                .with_form(FormElicitationCapability::new().with_schema_validation(true)),
        );
        info
    }

    /// Legacy `elicitation/create`: only valid inside an associated call whose
    /// token is still active; otherwise the request is rejected as unsolicited.
    /// Server-request cancellation resolves the form as cancelled rather than
    /// leaving it pending.
    async fn create_elicitation(
        &self,
        params: ElicitRequestParams,
        context: RequestContext<RoleClient>,
    ) -> Result<ElicitResult, ErrorData> {
        {
            let (message, schema) = match params {
                ElicitRequestParams::FormElicitationParams {
                    message,
                    requested_schema,
                    ..
                } => (message, requested_schema),
                ElicitRequestParams::UrlElicitationParams { .. } => {
                    return Err(ErrorData::invalid_params(
                        "URL-mode elicitation is not supported by this client.",
                        None,
                    ))
                }
                _ => {
                    return Err(ErrorData::invalid_params(
                        "Unsupported elicitation request mode.",
                        None,
                    ))
                }
            };
            let token = self.session.active_token().ok_or_else(|| {
                ErrorData::invalid_params(
                    "Elicitation arrived outside any associated operation on this connector.",
                    None,
                )
            })?;
            tokio::select! {
                result = self.session.ask(token, &message, &schema) => {
                    result.map_err(|error| ErrorData::internal_error(error, None))
                }
                _ = context.ct.cancelled() => Ok(ElicitResult::new(ElicitationAction::Cancel)),
            }
        }
    }
}

/// Normalize a bounded [`ElicitationSchema`] into the form spec sent to the
/// dialog. Anything outside the supported flat primitive/enum subset — or a
/// request for credentials — is rejected before a form is ever shown.
fn form_spec(message: &str, schema: &ElicitationSchema) -> Result<Value, String> {
    if message.len() > MAX_MESSAGE_BYTES {
        return Err("The server input message exceeds the 4 KiB limit.".into());
    }
    if schema.properties.len() > MAX_FORM_FIELDS {
        return Err(format!(
            "The server requested more than {MAX_FORM_FIELDS} form fields."
        ));
    }
    let required: HashSet<&str> = schema
        .required
        .iter()
        .flatten()
        .map(String::as_str)
        .collect();
    let order: Vec<String> = schema
        .property_order
        .clone()
        .unwrap_or_else(|| schema.properties.keys().cloned().collect());
    let mut fields = Vec::new();
    for name in order {
        let Some(definition) = schema.properties.get(&name) else {
            continue;
        };
        if name.len() > 256 {
            return Err("The server sent an oversized form field name.".into());
        }
        fields.push(field_spec(
            &name,
            required.contains(name.as_str()),
            definition,
        )?);
    }
    for (name, definition) in &schema.properties {
        if !fields
            .iter()
            .any(|field| field["name"].as_str() == Some(name.as_str()))
        {
            fields.push(field_spec(
                name,
                required.contains(name.as_str()),
                definition,
            )?);
        }
    }
    Ok(json!({"message": message, "fields": fields}))
}

fn text(value: &Option<std::borrow::Cow<'static, str>>) -> Result<Option<String>, String> {
    match value {
        Some(text) if text.len() > MAX_LABEL_BYTES => {
            Err("The server sent an oversized form label.".into())
        }
        Some(text) => Ok(Some(text.to_string())),
        None => Ok(None),
    }
}

fn sensitive(text: &str) -> bool {
    let lower = text.to_lowercase();
    // Credential-like words are matched on boundaries so `api_token` and
    // `access_token` are caught while `tokenizer` is not.
    const WORDS: &[&str] = &[
        "password",
        "passphrase",
        "passwd",
        "secret",
        "token",
        "credential",
        "credentials",
        "cvv",
        "ssn",
        "pin",
        "apikey",
    ];
    if lower
        .split(|c: char| !c.is_ascii_alphanumeric())
        .any(|word| WORDS.contains(&word))
    {
        return true;
    }
    [
        "api key",
        "apikey",
        "card number",
        "cardnumber",
        "social security",
        "private key",
        "privatekey",
        "pin code",
        "pincode",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

fn check_sensitive(name: &str, title: Option<&str>) -> Result<(), String> {
    if sensitive(name) || title.is_some_and(sensitive) {
        return Err(format!("The field '{name}' asks for a password, token or payment credential, which this form cannot accept."));
    }
    Ok(())
}

fn options(pairs: Vec<(String, Option<String>)>) -> Result<Vec<Value>, String> {
    if pairs.len() > MAX_OPTIONS {
        return Err(format!(
            "The server sent more than {MAX_OPTIONS} options for one field."
        ));
    }
    Ok(pairs
        .into_iter()
        .map(|(value, label)| {
            let mut option = json!({"value": value});
            if let Some(label) = label {
                if label.len() > MAX_LABEL_BYTES {
                    option["label"] =
                        json!(label.chars().take(MAX_LABEL_BYTES).collect::<String>());
                } else {
                    option["label"] = json!(label);
                }
            }
            option
        })
        .collect())
}

fn field_spec(
    name: &str,
    required: bool,
    definition: &PrimitiveSchemaDefinition,
) -> Result<Value, String> {
    let mut field = json!({"name": name, "required": required});
    let (title, description) = match definition {
        PrimitiveSchemaDefinition::String(schema) => {
            field["kind"] = json!("text");
            if let Some(format) = &schema.format {
                field["format"] = serde_json::to_value(format).unwrap_or_default();
            }
            if let Some(min) = schema.min_length {
                field["minLength"] = json!(min);
            }
            if let Some(max) = schema.max_length {
                field["maxLength"] = json!(max);
            }
            if let Some(default) = &schema.default {
                field["default"] = json!(default);
            }
            (text(&schema.title)?, text(&schema.description)?)
        }
        PrimitiveSchemaDefinition::Number(schema) => {
            field["kind"] = json!("number");
            if let Some(min) = schema.minimum {
                field["minimum"] = json!(min);
            }
            if let Some(max) = schema.maximum {
                field["maximum"] = json!(max);
            }
            if let Some(default) = schema.default {
                field["default"] = json!(default);
            }
            (text(&schema.title)?, text(&schema.description)?)
        }
        PrimitiveSchemaDefinition::Integer(schema) => {
            field["kind"] = json!("integer");
            if let Some(min) = schema.minimum {
                field["minimum"] = json!(min);
            }
            if let Some(max) = schema.maximum {
                field["maximum"] = json!(max);
            }
            if let Some(default) = schema.default {
                field["default"] = json!(default);
            }
            (text(&schema.title)?, text(&schema.description)?)
        }
        PrimitiveSchemaDefinition::Boolean(schema) => {
            // Optional booleans render as an explicit tri-state so unanswered
            // never silently becomes false.
            field["kind"] = json!("boolean");
            if let Some(default) = schema.default {
                field["default"] = json!(default);
            }
            (text(&schema.title)?, text(&schema.description)?)
        }
        PrimitiveSchemaDefinition::Enum(schema) => match schema {
            EnumSchema::Single(single) => {
                field["kind"] = json!("select");
                let (pairs, meta) = match single {
                    SingleSelectEnumSchema::Untitled(schema) => (
                        schema
                            .enum_
                            .iter()
                            .map(|value| (value.clone(), None))
                            .collect(),
                        (
                            text(&schema.title)?,
                            text(&schema.description)?,
                            schema.default.clone(),
                        ),
                    ),
                    SingleSelectEnumSchema::Titled(schema) => (
                        schema
                            .one_of
                            .iter()
                            .map(|item| (item.const_.clone(), Some(item.title.clone())))
                            .collect(),
                        (
                            text(&schema.title)?,
                            text(&schema.description)?,
                            schema.default.clone(),
                        ),
                    ),
                    _ => return Err("The server sent an unsupported select field shape.".into()),
                };
                field["options"] = json!(options(pairs)?);
                if let Some(default) = meta.2 {
                    field["default"] = json!(default);
                }
                (meta.0, meta.1)
            }
            EnumSchema::Multi(multi) => {
                field["kind"] = json!("multi");
                let (pairs, meta) = match multi {
                    MultiSelectEnumSchema::Untitled(schema) => (
                        schema
                            .items
                            .enum_
                            .iter()
                            .map(|value| (value.clone(), None))
                            .collect(),
                        (
                            text(&schema.title)?,
                            text(&schema.description)?,
                            schema.min_items,
                            schema.max_items,
                            schema.default.clone(),
                        ),
                    ),
                    MultiSelectEnumSchema::Titled(schema) => (
                        schema
                            .items
                            .any_of
                            .iter()
                            .map(|item| (item.const_.clone(), Some(item.title.clone())))
                            .collect(),
                        (
                            text(&schema.title)?,
                            text(&schema.description)?,
                            schema.min_items,
                            schema.max_items,
                            schema.default.clone(),
                        ),
                    ),
                    _ => {
                        return Err(
                            "The server sent an unsupported multi-select field shape.".into()
                        )
                    }
                };
                field["options"] = json!(options(pairs)?);
                if let Some(min) = meta.2 {
                    field["minItems"] = json!(min);
                }
                if let Some(max) = meta.3 {
                    field["maxItems"] = json!(max);
                }
                if let Some(default) = meta.4 {
                    field["default"] = json!(default);
                }
                (meta.0, meta.1)
            }
            EnumSchema::Legacy(schema) => {
                field["kind"] = json!("select");
                let pairs: Vec<(String, Option<String>)> = schema
                    .enum_
                    .iter()
                    .enumerate()
                    .map(|(index, value)| {
                        (
                            value.clone(),
                            schema
                                .enum_names
                                .as_ref()
                                .and_then(|names| names.get(index).cloned()),
                        )
                    })
                    .collect();
                field["options"] = json!(options(pairs)?);
                if let Some(default) = &schema.default {
                    field["default"] = json!(default);
                }
                (text(&schema.title)?, text(&schema.description)?)
            }
            _ => return Err("The server sent an unsupported enum field shape.".into()),
        },
        _ => return Err("The server sent an unsupported form field type.".into()),
    };
    check_sensitive(name, title.as_deref())?;
    if let Some(title) = title {
        field["title"] = json!(title);
    }
    if let Some(description) = description {
        field["description"] = json!(description);
    }
    Ok(field)
}

/// Tauri command: the only resolver for typed MCP input. Invalid accepts keep
/// the request pending; unknown ids fail closed.
#[tauri::command]
pub async fn resolve_mcp_input(
    state: tauri::State<'_, crate::AppState>,
    request_id: String,
    action: String,
    content: Option<Value>,
) -> Result<(), String> {
    state.mcp_inputs.resolve(&request_id, &action, content)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a schema the way a server sends it: as raw JSON, exercising the
    /// same wire deserialization the handler path uses.
    fn schema_with(properties: Value, required: Vec<&str>) -> ElicitationSchema {
        serde_json::from_value(json!({
            "type": "object",
            "properties": properties,
            "required": required,
        }))
        .expect("fixture schema should deserialize")
    }

    #[test]
    fn form_spec_rejects_sensitive_fields() {
        let schema = schema_with(json!({"api_token": {"type": "string"}}), vec![]);
        let error = form_spec("Enter details", &schema).unwrap_err();
        assert!(error.contains("credential"), "{error}");
    }

    #[test]
    fn form_spec_flattens_supported_fields() {
        let schema = schema_with(
            json!({
                "name": {"type": "string", "title": "Name"},
                "enabled": {"type": "boolean"},
                "color": {"type": "string", "enum": ["red", "blue"]},
                "tags": {"type": "array", "items": {"type": "string", "enum": ["a", "b"]}},
            }),
            vec!["name"],
        );
        let spec = form_spec("Who are you?", &schema).unwrap();
        assert_eq!(spec["message"], "Who are you?");
        let fields = spec["fields"].as_array().unwrap();
        assert_eq!(fields.len(), 4);
        // serde_json maps sort keys, so the wire order is alphabetical.
        assert_eq!(fields[0]["name"], "color");
        assert_eq!(fields[0]["kind"], "select");
        assert_eq!(fields[1]["kind"], "boolean");
        assert_eq!(fields[2]["name"], "name");
        assert_eq!(fields[2]["required"], true);
        assert_eq!(fields[3]["kind"], "multi");
        assert_eq!(fields[3]["options"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn registry_rejects_wrong_action_and_keeps_pending() {
        let registry = Registry::default();
        let schema = serde_json::to_value(schema_with(
            json!({"name": {"type": "string"}}),
            vec!["name"],
        ))
        .unwrap();
        let (id, _receiver) = registry.begin(7, schema).unwrap();
        assert!(registry.resolve(&id, "bogus", None).is_err());
        assert!(registry
            .resolve(&id, "accept", Some(json!({"other": 1})))
            .is_err());
        assert!(registry.resolve(&id, "accept", Some(json!({}))).is_err());
        assert!(registry
            .resolve(&id, "accept", Some(json!({"name": "Ada"})))
            .is_ok());
        assert!(registry
            .resolve(&id, "accept", Some(json!({"name": "Ada"})))
            .is_err());
    }

    #[test]
    fn registry_scope_removal_closes_waiters() {
        let registry = Registry::default();
        let schema = serde_json::to_value(schema_with(json!({}), vec![])).unwrap();
        let (id, receiver) = registry.begin(9, schema).unwrap();
        assert_eq!(registry.remove_scope(9), vec![id]);
        assert!(receiver.blocking_recv().is_err());
    }

    #[tokio::test]
    async fn scope_ids_are_unique_across_connector_sessions() {
        // The pending registry is global: two connectors must never share a
        // scope id, or one connector's cleanup cancels the other's form.
        let registry = Arc::new(Registry::default());
        let first = SessionInput::new("one".into(), None);
        let second = SessionInput::new("two".into(), None);
        let ui = CallUi::new(registry.clone(), Arc::new(|_message, _form| {}));
        let scope_a = first.enter(Some(ui.clone()), "call").await.unwrap();
        let scope_b = second.enter(Some(ui), "call").await.unwrap();
        let (token_a, token_b) = (scope_a.token, scope_b.token);
        assert_ne!(token_a, token_b);
        // The session gate serializes the calls of one session, so the first
        // scope has to be released before the next call on that session.
        drop(scope_a);
        drop(scope_b);
        let scope_c = first.enter(None, "call").await.unwrap();
        assert_ne!(scope_c.token, token_a);
        assert_ne!(scope_c.token, token_b);
    }

    #[test]
    fn removing_one_connector_scope_leaves_another_connectors_form_pending() {
        let registry = Registry::default();
        let schema = serde_json::to_value(schema_with(json!({}), vec![])).unwrap();
        let first = mint_scope();
        let second = mint_scope();
        assert_ne!(first, second);
        let (first_id, _first_receiver) = registry.begin(first, schema.clone()).unwrap();
        let (second_id, _second_receiver) = registry.begin(second, schema).unwrap();
        assert_eq!(registry.remove_scope(first), vec![first_id]);
        assert!(
            lock(&registry.pending).contains_key(&second_id),
            "another connector's pending form must survive"
        );
        assert_eq!(registry.remove_scope(second), vec![second_id]);
    }
}

//! Authoring-model providers and the judged-fidelity judge (FEAT-023, D-005).
//!
//! The harness reaches an authoring model through one of the pinned providers
//! the stack records — OpenAI, Anthropic, Google and the OpenCode adapter — or
//! through the offline `replay` provider, which reads recorded model replies so
//! a run is reproducible with no network. A provider is chosen on the command
//! line as `provider:model`; the model string is the pinned model version the
//! score applies to.
//!
//! Credentials are read from the provider's documented environment variable and
//! are never written to the record, the log or the repository (NFR-026). The
//! harness makes a network call only for the model the maintainer names; Vectr
//! never calls a model on behalf of a scene (NFR-024, C-006).
//!
//! The OpenCode adapter (D-049) runs the authoring call through a minimal,
//! tool-free OpenCode agent and reads that call's token usage back from the
//! OpenCode session, so the reported count is the authoring call's usage rather
//! than OpenCode's own agent prompt and tool definitions. Without that, each
//! call would process tens of thousands of tokens unrelated to Vectr and breach
//! NFR-030's per-prompt budget.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use serde_json::{json, Value};

use crate::corpus::{Prompt, Quality};
use crate::json::extract_json;
use crate::toolchain::PALETTE_TOKENS;

/// The versioned authoring prompt artifact: the system prompt the harness
/// sends with the published schema and the packaged guidance.
const AUTHORING_PROMPT: &str = include_str!("../prompts/authoring.v2.txt");

/// The skill's always-read entry point (FEAT-020).
///
/// The docs promise the authoring input is the published schema and the skill's
/// instructions, so the harness carries the skill rather than the scaffolded
/// project guide. The entry point is the always-read surface and routes a
/// coding agent to its on-demand references; a one-shot authoring call cannot
/// load a reference when a step needs it, so the rules the authoring step turns
/// on are stated in the versioned authoring prompt (`authoring.v2.txt`) rather
/// than preloading the whole reference depth and paying for it in every prompt's
/// context (FEAT-020, D-042). The evaluation crate is maintainer-only and
/// unpublished, so it embeds the shipped skill from the repository rather than
/// carrying a second copy that could drift.
const SKILL_ENTRY: &str = include_str!("../../../skills/vectr/SKILL.md");

/// The versioned judge prompt artifact.
const JUDGE_PROMPT: &str = include_str!("../prompts/judge.v1.txt");

/// The environment variable holding the OpenAI API key.
pub const OPENAI_KEY: &str = "OPENAI_API_KEY";
/// The environment variable holding the Anthropic API key.
pub const ANTHROPIC_KEY: &str = "ANTHROPIC_API_KEY";
/// The environment variables holding a Google API key.
pub const GOOGLE_KEYS: [&str; 2] = ["GOOGLE_API_KEY", "GEMINI_API_KEY"];
/// The environment variable overriding the OpenCode binary.
pub const OPENCODE_BIN: &str = "VECTR_OPENCODE_BIN";

/// The name of the minimal, tool-free OpenCode agent the adapter selects. The
/// agent is declared in a per-call scratch project and carries the call's system
/// prompt, so the authoring call processes the Vectr system prompt rather than
/// OpenCode's own (D-049).
const OPENCODE_AGENT: &str = "vectr-eval";

/// One of the pinned provider families.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderKind {
    /// OpenAI's chat completions API.
    OpenAi,
    /// Anthropic's messages API.
    Anthropic,
    /// Google's generateContent API.
    Google,
    /// The OpenCode adapter, run as a local subprocess.
    OpenCode,
    /// Recorded replies read from a directory, for offline runs.
    Replay,
}

impl ProviderKind {
    /// The provider's name on the command line.
    pub fn as_str(self) -> &'static str {
        match self {
            ProviderKind::OpenAi => "openai",
            ProviderKind::Anthropic => "anthropic",
            ProviderKind::Google => "google",
            ProviderKind::OpenCode => "opencode",
            ProviderKind::Replay => "replay",
        }
    }

    fn parse(name: &str) -> Option<Self> {
        match name {
            "openai" => Some(ProviderKind::OpenAi),
            "anthropic" => Some(ProviderKind::Anthropic),
            "google" => Some(ProviderKind::Google),
            "opencode" => Some(ProviderKind::OpenCode),
            "replay" => Some(ProviderKind::Replay),
            _ => None,
        }
    }
}

/// A provider and the pinned model it runs, parsed from `provider:model`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelSpec {
    /// The provider family.
    pub provider: ProviderKind,
    /// The model (or, for `replay`, the directory of recorded replies).
    pub model: String,
}

impl ModelSpec {
    /// The run's `authoringModelId`, `provider:model`.
    pub fn id(&self) -> String {
        format!("{}:{}", self.provider.as_str(), self.model)
    }
}

/// Parses `provider:model`, naming the providers when the name is unknown.
pub fn parse_model_spec(spec: &str) -> Result<ModelSpec, String> {
    let (provider, model) = spec
        .split_once(':')
        .ok_or_else(|| format!("`{spec}` is not a `provider:model` reference"))?;
    if model.is_empty() {
        return Err(format!("`{spec}` names no model"));
    }
    let provider = ProviderKind::parse(provider).ok_or_else(|| {
        format!(
            "unknown provider `{provider}`; expected openai, anthropic, google, opencode or replay"
        )
    })?;
    Ok(ModelSpec {
        provider,
        model: model.to_string(),
    })
}

/// A provider could not be reached, or its reply could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderError(pub String);

impl std::fmt::Display for ProviderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ProviderError {}

/// A model's reply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Authored {
    /// The reply text, expected to hold the authored JSON.
    pub text: String,
    /// The reply's token usage, when the provider reports it.
    pub tokens: Option<u64>,
}

/// A judge's verdict on one authored scene.
#[derive(Debug, Clone, PartialEq)]
pub struct Judgement {
    /// Judged fidelity in `[0, 1]`.
    pub fidelity: f64,
    /// Whether each quality was demonstrated, keyed by its stable key.
    pub qualities: BTreeMap<String, bool>,
}

/// Authors scenes from natural-language prompts.
pub trait AuthoringModel {
    /// The model's `provider:model` label.
    fn label(&self) -> &str;
    /// Authors the scene a prompt asks for.
    fn author(&self, prompt: &Prompt) -> Result<Authored, ProviderError>;
}

/// Judges how well an authored scene realizes a prompt.
pub trait Judge {
    /// Scores one authored scene.
    fn judge(&self, prompt: &Prompt, authored: &Authored) -> Result<Judgement, ProviderError>;
}

/// A synchronous chat completion, the seam every HTTP and subprocess provider
/// shares.
trait Completion {
    fn complete(&self, system: &str, user: &str) -> Result<Authored, ProviderError>;
}

/// The authoring model and judge a run uses.
pub type ModelPair = (Box<dyn AuthoringModel>, Box<dyn Judge>);

/// Builds the authoring model and the judge a run uses.
///
/// For `replay`, both read recorded replies from the directory the model string
/// names. For every other provider, both wrap the same provider client: the
/// authoring model uses the versioned authoring prompt, the judge the versioned
/// judge prompt.
pub fn build(spec: &ModelSpec) -> Result<ModelPair, ProviderError> {
    if spec.provider == ProviderKind::Replay {
        let dir = PathBuf::from(&spec.model);
        if !dir.is_dir() {
            return Err(ProviderError(format!(
                "the replay provider expects a directory of recorded replies, but `{}` is not a directory",
                dir.display()
            )));
        }
        return Ok((
            Box::new(ReplayAuthor { dir: dir.clone() }),
            Box::new(ReplayJudge { dir }),
        ));
    }

    let author_client = build_client(spec)?;
    let judge_client = build_client(spec)?;
    Ok((
        Box::new(LlmAuthor {
            label: spec.id(),
            client: Box::new(RetryingClient::new(author_client)),
        }),
        Box::new(LlmJudge {
            client: Box::new(RetryingClient::new(judge_client)),
        }),
    ))
}

fn build_client(spec: &ModelSpec) -> Result<Box<dyn Completion>, ProviderError> {
    match spec.provider {
        ProviderKind::OpenAi | ProviderKind::Anthropic | ProviderKind::Google => {
            let key = api_key(spec.provider)?;
            Ok(Box::new(HttpModel {
                kind: spec.provider,
                model: spec.model.clone(),
                key,
                client: reqwest::blocking::Client::new(),
            }))
        }
        ProviderKind::OpenCode => Ok(Box::new(OpenCodeModel {
            model: spec.model.clone(),
            bin: std::env::var(OPENCODE_BIN).unwrap_or_else(|_| "opencode".to_string()),
        })),
        ProviderKind::Replay => Err(ProviderError(
            "the replay provider is not an HTTP client".to_string(),
        )),
    }
}

/// Wraps a provider client so a transiently failed call is retried with growing
/// backoff before the caller sees the failure (D-052, D-053).
///
/// It wraps the authoring call and the judge call alike, so a hiccup on either
/// no longer discards the prompt: an authoring call that exhausts its retries
/// abandons the run, while a judge call that does leaves its prompt unscorable.
/// The offline `replay` provider is not wrapped, so a missing recording stays
/// an immediate, deterministic failure.
struct RetryingClient {
    inner: Box<dyn Completion>,
    attempts: usize,
    backoff: Duration,
}

impl RetryingClient {
    /// Wraps `inner` with the harness's bounded, growing-backoff retry.
    fn new(inner: Box<dyn Completion>) -> Self {
        Self {
            inner,
            attempts: RETRY_ATTEMPTS,
            backoff: RETRY_BACKOFF,
        }
    }
}

impl Completion for RetryingClient {
    fn complete(&self, system: &str, user: &str) -> Result<Authored, ProviderError> {
        retrying(self.attempts, self.backoff, |_attempt| {
            self.inner.complete(system, user)
        })
    }
}

fn api_key(kind: ProviderKind) -> Result<String, ProviderError> {
    let (names, hint) = match kind {
        ProviderKind::OpenAi => (vec![OPENAI_KEY], OPENAI_KEY),
        ProviderKind::Anthropic => (vec![ANTHROPIC_KEY], ANTHROPIC_KEY),
        ProviderKind::Google => (GOOGLE_KEYS.to_vec(), "GOOGLE_API_KEY"),
        _ => return Err(ProviderError("this provider takes no API key".to_string())),
    };
    for name in names {
        if let Ok(value) = std::env::var(name) {
            if !value.is_empty() {
                return Ok(value);
            }
        }
    }
    Err(ProviderError(format!(
        "the {} provider needs its API key in {hint}",
        kind.as_str()
    )))
}

/// The versioned authoring system prompt: the instructions, the published
/// schema, the skill's entry point, and the style assets the evaluation project
/// supplies (FEAT-020, FEAT-032 Model Behavior).
pub fn authoring_system() -> String {
    let schema = vectr_core::schema(vectr_core::SchemaForm::Compact).unwrap_or_default();
    let tokens: Vec<String> = PALETTE_TOKENS
        .iter()
        .map(|(name, value)| format!("- {name}: {value}"))
        .collect();
    format!(
        "{AUTHORING_PROMPT}\n\n\
         # Skill entry point\n\n{SKILL_ENTRY}\n\n\
         # Published schema (compact)\n\n{schema}\n\n\
         # Available palette tokens\n\n{}\n\n\
         # Available stroke profiles\n\n\
         (none — draw with token fills and set no `stroke`)\n\n\
         # Available gradients\n\n(none)\n",
        tokens.join("\n")
    )
}

/// The judge's user message: the intent, the request and the authored scene.
fn judge_user(prompt: &Prompt, authored: &Authored) -> String {
    format!(
        "# Intent\n\n{}\n\n# Request\n\n{}\n\n# Authored scene\n\n{}",
        prompt.intent, prompt.text, authored.text
    )
}

struct LlmAuthor {
    label: String,
    client: Box<dyn Completion>,
}

impl AuthoringModel for LlmAuthor {
    fn label(&self) -> &str {
        &self.label
    }

    fn author(&self, prompt: &Prompt) -> Result<Authored, ProviderError> {
        self.client.complete(&authoring_system(), &prompt.text)
    }
}

struct LlmJudge {
    client: Box<dyn Completion>,
}

impl Judge for LlmJudge {
    fn judge(&self, prompt: &Prompt, authored: &Authored) -> Result<Judgement, ProviderError> {
        let user = judge_user(prompt, authored);
        let reply = self.client.complete(JUDGE_PROMPT, &user)?;
        parse_judgement(&reply.text).map_err(ProviderError)
    }
}

struct ReplayAuthor {
    dir: PathBuf,
}

impl AuthoringModel for ReplayAuthor {
    fn label(&self) -> &str {
        "replay"
    }

    fn author(&self, prompt: &Prompt) -> Result<Authored, ProviderError> {
        let path = self.dir.join(format!("{}.scene.json", prompt.id));
        let text = std::fs::read_to_string(&path).map_err(|error| {
            ProviderError(format!(
                "the replay provider has no recorded authoring output for prompt `{}` at `{}`: {error}",
                prompt.id,
                path.display()
            ))
        })?;
        Ok(Authored {
            text,
            tokens: recorded_tokens(&self.dir, &prompt.id)?,
        })
    }
}

/// Reads a recorded token usage, when the recording carries one. A
/// `<id>.usage.json` file holds `{"tokens": <count>}`; a missing file means the
/// provider reported no usage, so the prompt is excluded from the run's median
/// rather than counted as zero (FEAT-023). A malformed recording is a failure,
/// never a silent absence (NFR-011).
fn recorded_tokens(dir: &Path, id: &str) -> Result<Option<u64>, ProviderError> {
    let path = dir.join(format!("{id}.usage.json"));
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            let message = format!(
                "the replay provider cannot read the recorded usage for prompt `{id}` at `{}`: {error}",
                path.display()
            );
            return Err(ProviderError(message));
        }
    };
    let value: Value = serde_json::from_str(text.trim()).map_err(|error| {
        ProviderError(format!(
            "the replay provider's recorded usage for prompt `{id}` at `{}` is not valid JSON: {error}",
            path.display()
        ))
    })?;
    value
        .get("tokens")
        .and_then(Value::as_u64)
        .map(Some)
        .ok_or_else(|| {
            ProviderError(format!(
                "the replay provider's recorded usage for prompt `{id}` at `{}` names no `tokens` count",
                path.display()
            ))
        })
}

struct ReplayJudge {
    dir: PathBuf,
}

impl Judge for ReplayJudge {
    fn judge(&self, prompt: &Prompt, _authored: &Authored) -> Result<Judgement, ProviderError> {
        let path = self.dir.join(format!("{}.judge.json", prompt.id));
        let text = std::fs::read_to_string(&path).map_err(|error| {
            ProviderError(format!(
                "the replay provider has no recorded judgement for prompt `{}` at `{}`: {error}",
                prompt.id,
                path.display()
            ))
        })?;
        parse_judgement(&text).map_err(ProviderError)
    }
}

/// Reads a prompt's recorded authoring reply from a run's log directory, when a
/// resumed run reuses it instead of calling the model again (D-053).
///
/// The artifacts are the ones a logging run writes and the `replay` provider
/// reads — `<id>.scene.json`, with an optional `<id>.usage.json`. `Ok(None)`
/// means the prompt has no recorded reply, so the model is called; a recording
/// that exists but cannot be read or parsed is reported, never silently skipped
/// (NFR-011).
pub fn read_recorded_authored(dir: &Path, id: &str) -> Result<Option<Authored>, ProviderError> {
    let path = dir.join(format!("{id}.scene.json"));
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(ProviderError(format!(
                "cannot read the recorded authoring output for prompt `{id}` at `{}`: {error}",
                path.display()
            )))
        }
    };
    Ok(Some(Authored {
        text,
        tokens: recorded_tokens(dir, id)?,
    }))
}

/// Reads a prompt's recorded judge verdict from a run's log directory, when a
/// resumed run reuses it instead of judging again (D-053).
///
/// `Ok(None)` means the prompt has no usable verdict recorded — the artifact is
/// absent, or it is the harness's own record of a judge that returned no
/// verdict — so the prompt is judged again. A recording that exists but cannot
/// be read is reported, never silently skipped (NFR-011).
pub fn read_recorded_judgement(dir: &Path, id: &str) -> Result<Option<Judgement>, ProviderError> {
    let path = dir.join(format!("{id}.judge.json"));
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(ProviderError(format!(
                "cannot read the recorded verdict for prompt `{id}` at `{}`: {error}",
                path.display()
            )))
        }
    };
    Ok(parse_judgement(&text).ok())
}

/// Parses a judge's verdict from its reply text.
pub fn parse_judgement(text: &str) -> Result<Judgement, String> {
    let value =
        extract_json(text).ok_or_else(|| "the judge returned no JSON object".to_string())?;
    let fidelity = value
        .get("fidelity")
        .and_then(Value::as_f64)
        .ok_or_else(|| "the judge returned no `fidelity` number".to_string())?
        .clamp(0.0, 1.0);
    let qualities = value.get("qualities").and_then(Value::as_object);
    let mut scored = BTreeMap::new();
    for quality in Quality::HARD_END {
        let passed = qualities
            .and_then(|map| map.get(quality.key()))
            .and_then(Value::as_bool)
            .unwrap_or(false);
        scored.insert(quality.key().to_string(), passed);
    }
    Ok(Judgement {
        fidelity,
        qualities: scored,
    })
}

/// An HTTP provider reached over the pinned vendor API.
struct HttpModel {
    kind: ProviderKind,
    model: String,
    key: String,
    client: reqwest::blocking::Client,
}

impl Completion for HttpModel {
    fn complete(&self, system: &str, user: &str) -> Result<Authored, ProviderError> {
        let body = build_body(self.kind, &self.model, system, user);
        let mut url = endpoint(self.kind, &self.model);
        // Google takes its key as a query parameter; the `query` feature is not
        // enabled, so it is appended here.
        if self.kind == ProviderKind::Google {
            url = format!("{url}?key={}", self.key);
        }
        let mut request = self.client.post(&url).json(&body);
        request = match self.kind {
            ProviderKind::OpenAi => request.bearer_auth(&self.key),
            ProviderKind::Anthropic => request
                .header("x-api-key", &self.key)
                .header("anthropic-version", "2023-06-01"),
            _ => request,
        };
        let response = request.send().map_err(|error| {
            ProviderError(format!(
                "the {} provider could not be reached: {error}",
                self.kind.as_str()
            ))
        })?;
        let status = response.status();
        let value: Value = response.json().map_err(|error| {
            ProviderError(format!(
                "the {} provider returned an unreadable response: {error}",
                self.kind.as_str()
            ))
        })?;
        if !status.is_success() {
            return Err(ProviderError(format!(
                "the {} provider returned {status}: {}",
                self.kind.as_str(),
                error_message(&value)
            )));
        }
        parse_body(self.kind, &value)
    }
}

/// The vendor API endpoint for a model.
pub fn endpoint(kind: ProviderKind, model: &str) -> String {
    match kind {
        ProviderKind::OpenAi => "https://api.openai.com/v1/chat/completions".to_string(),
        ProviderKind::Anthropic => "https://api.anthropic.com/v1/messages".to_string(),
        ProviderKind::Google => format!(
            "https://generativelanguage.googleapis.com/v1beta/models/{model}:generateContent"
        ),
        _ => String::new(),
    }
}

/// The request body a vendor expects, with decoding pinned to temperature zero
/// so repeated runs are reproducible (FEAT-023).
pub fn build_body(kind: ProviderKind, model: &str, system: &str, user: &str) -> Value {
    match kind {
        ProviderKind::OpenAi => json!({
            "model": model,
            "temperature": 0,
            "response_format": { "type": "json_object" },
            "messages": [
                { "role": "system", "content": system },
                { "role": "user", "content": user }
            ]
        }),
        ProviderKind::Anthropic => json!({
            "model": model,
            "max_tokens": 8192,
            "temperature": 0,
            "system": system,
            "messages": [{ "role": "user", "content": user }]
        }),
        ProviderKind::Google => json!({
            "systemInstruction": { "parts": [{ "text": system }] },
            "contents": [{ "role": "user", "parts": [{ "text": user }] }],
            "generationConfig": { "temperature": 0, "responseMimeType": "application/json" }
        }),
        _ => Value::Null,
    }
}

/// Reads a vendor's reply text and token usage.
pub fn parse_body(kind: ProviderKind, value: &Value) -> Result<Authored, ProviderError> {
    let (text, tokens) = match kind {
        ProviderKind::OpenAi => (
            value
                .pointer("/choices/0/message/content")
                .and_then(Value::as_str),
            value.pointer("/usage/total_tokens").and_then(Value::as_u64),
        ),
        ProviderKind::Anthropic => (
            value.pointer("/content/0/text").and_then(Value::as_str),
            sum_tokens(value, "/usage/input_tokens", "/usage/output_tokens"),
        ),
        ProviderKind::Google => (
            value
                .pointer("/candidates/0/content/parts/0/text")
                .and_then(Value::as_str),
            value
                .pointer("/usageMetadata/totalTokenCount")
                .and_then(Value::as_u64),
        ),
        _ => (None, None),
    };
    let text = text.ok_or_else(|| {
        ProviderError(format!(
            "the {} provider's reply carried no text",
            kind.as_str()
        ))
    })?;
    Ok(Authored {
        text: text.to_string(),
        tokens,
    })
}

fn sum_tokens(value: &Value, input: &str, output: &str) -> Option<u64> {
    let a = value.pointer(input).and_then(Value::as_u64);
    let b = value.pointer(output).and_then(Value::as_u64);
    match (a, b) {
        (Some(a), Some(b)) => Some(a + b),
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (None, None) => None,
    }
}

fn error_message(value: &Value) -> String {
    value
        .pointer("/error/message")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| value.to_string())
}

/// The OpenCode adapter, run as a local subprocess so it uses the user's own
/// OpenCode authentication and reaches every model OpenCode exposes (D-005).
///
/// Each call runs in a throwaway OpenCode project whose configuration declares a
/// single tool-free agent carrying the call's system prompt. The call's token
/// usage is then read back from the session the run created (D-049).
struct OpenCodeModel {
    model: String,
    bin: String,
}

/// How many times a transiently failed provider call is attempted — the first
/// attempt plus two retries — before the caller sees the failure (D-052,
/// D-053).
const RETRY_ATTEMPTS: usize = 3;

/// The wait before the first retry of a provider call. Each later retry waits
/// twice as long as the one before, so a transient outage is absorbed without a
/// tight retry loop (D-053).
const RETRY_BACKOFF: Duration = Duration::from_millis(500);

/// One OpenCode run's event stream, decoded: the session it created and the
/// reply text it streamed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenCodeRun {
    /// The session identifier the run created, used to read its usage.
    pub session: String,
    /// The reply text the run streamed.
    pub reply: String,
}

impl Completion for OpenCodeModel {
    fn complete(&self, system: &str, user: &str) -> Result<Authored, ProviderError> {
        let project = OpenCodeProject::create(system).map_err(ProviderError)?;
        let run = self.run(&project, user)?;
        // The reply comes from the run event stream, so an unreadable or
        // truncated session-export usage read is no usage rather than a failed
        // call (D-052).
        let tokens = self
            .export(&project, &run.session)
            .ok()
            .and_then(|export| parse_opencode_usage(&export));
        Ok(Authored {
            text: run.reply,
            tokens,
        })
    }
}

/// Calls `call` up to `attempts` times, returning the first success or the last
/// failure. A failed attempt waits `backoff` doubled once for each earlier
/// attempt before the next, so a transient provider or server hiccup is
/// absorbed with growing backoff and never an unbounded retry loop (D-052,
/// D-053).
fn retrying<T>(
    attempts: usize,
    backoff: Duration,
    mut call: impl FnMut(usize) -> Result<T, ProviderError>,
) -> Result<T, ProviderError> {
    let attempts = attempts.max(1);
    let mut last = None;
    for attempt in 1..=attempts {
        match call(attempt) {
            Ok(value) => return Ok(value),
            Err(error) => {
                last = Some(error);
                if attempt < attempts {
                    let wait = retry_delay(backoff, attempt);
                    if !wait.is_zero() {
                        std::thread::sleep(wait);
                    }
                }
            }
        }
    }
    Err(last.expect("at least one attempt was made"))
}

/// The wait before the attempt that follows `number` failed attempts: the base
/// delay doubled once for each attempt already made, so the waits grow
/// geometrically and saturate rather than overflow (D-053).
fn retry_delay(backoff: Duration, number: usize) -> Duration {
    let mut delay = backoff;
    for _ in 1..number {
        delay = delay.saturating_mul(2);
    }
    delay
}

impl OpenCodeModel {
    /// Runs one authoring call and returns the event stream, decoded.
    ///
    /// OpenCode resolves the project from `PWD` rather than the process working
    /// directory, and a subprocess's `PWD` is not updated by `current_dir`, so
    /// both are set to the scratch project; otherwise OpenCode searches the
    /// harness's own directory for the agent and does not find it.
    fn run(&self, project: &OpenCodeProject, user: &str) -> Result<OpenCodeRun, ProviderError> {
        let output = Command::new(&self.bin)
            .current_dir(project.path())
            .env("PWD", project.path())
            .args(opencode_run_args(&self.model, user))
            .output()
            .map_err(|error| {
                ProviderError(format!(
                    "the OpenCode adapter could not run `{}`: {error}",
                    self.bin
                ))
            })?;
        if !output.status.success() {
            return Err(ProviderError(format!(
                "the OpenCode adapter failed for model `{}`: {}",
                self.model,
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }
        parse_opencode_run(&String::from_utf8_lossy(&output.stdout)).map_err(ProviderError)
    }

    /// Reads a session's messages, which carry the call's usage.
    fn export(&self, project: &OpenCodeProject, session: &str) -> Result<String, ProviderError> {
        let output = Command::new(&self.bin)
            .current_dir(project.path())
            .env("PWD", project.path())
            .args(["session", "export", session])
            .output()
            .map_err(|error| {
                ProviderError(format!(
                    "the OpenCode adapter could not read session usage via `{}`: {error}",
                    self.bin
                ))
            })?;
        if !output.status.success() {
            return Err(ProviderError(format!(
                "the OpenCode adapter could not export session `{session}`: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }
}

/// The arguments for one authoring call: the JSON event stream, the minimal
/// tool-free agent, the pinned model, a fixed title so OpenCode makes no
/// title-generation call of its own, and the message (`--` keeps a message that
/// begins with a dash from reading as a flag).
pub fn opencode_run_args(model: &str, message: &str) -> Vec<String> {
    [
        "run",
        "--format",
        "json",
        "--agent",
        OPENCODE_AGENT,
        "--model",
        model,
        "--title",
        "vectr-eval",
        "--",
        message,
    ]
    .into_iter()
    .map(str::to_string)
    .collect()
}

/// A throwaway OpenCode project holding the tool-free agent a call runs
/// through. It is created under the system temporary directory and removed when
/// the call ends, so it never touches the harness's own project or the user's
/// OpenCode configuration.
struct OpenCodeProject {
    dir: PathBuf,
}

impl OpenCodeProject {
    /// Creates a project whose configuration declares the tool-free agent with
    /// `system` as its system prompt.
    fn create(system: &str) -> Result<Self, String> {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "vectr-eval-opencode-{}-{unique}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).map_err(|error| {
            format!(
                "the OpenCode adapter could not create its scratch project at `{}`: {error}",
                dir.display()
            )
        })?;
        let config = dir.join("opencode.json");
        std::fs::write(&config, opencode_config(system)).map_err(|error| {
            format!(
                "the OpenCode adapter could not write its configuration at `{}`: {error}",
                config.display()
            )
        })?;
        Ok(Self { dir })
    }

    /// The project directory.
    fn path(&self) -> &Path {
        &self.dir
    }
}

impl Drop for OpenCodeProject {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// The OpenCode project configuration that declares one tool-free, primary agent
/// carrying `system` as its prompt. Denying every permission keeps tools out of
/// the call's context.
pub fn opencode_config(system: &str) -> String {
    let agent = json!({
        "description": "Vectr evaluation authoring model",
        "mode": "primary",
        "system": system,
        "permissions": [{ "action": "*", "resource": "*", "effect": "deny" }],
    });
    let mut agents = serde_json::Map::new();
    agents.insert(OPENCODE_AGENT.to_string(), agent);
    serde_json::to_string_pretty(&json!({ "agents": agents }))
        .expect("the OpenCode config is serializable")
}

/// Reads the session identifier and the streamed reply out of
/// `opencode run --format json` output.
///
/// The stream is newline-delimited JSON events; each names the session, and the
/// `text` events carry the reply, one full text part each. An error event means
/// the call failed and is reported with its message. A run that names no
/// session is a failure, never a silent absence (NFR-011, D-052).
pub fn parse_opencode_run(stdout: &str) -> Result<OpenCodeRun, String> {
    let mut session: Option<String> = None;
    let mut reply = String::new();
    for line in stdout.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(event) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let kind = event.get("type").and_then(Value::as_str);
        if kind == Some("error") {
            let message = event
                .pointer("/error/message")
                .and_then(Value::as_str)
                .unwrap_or("unknown error");
            return Err(format!("the OpenCode adapter reported an error: {message}"));
        }
        if session.is_none() {
            if let Some(id) = event.get("sessionID").and_then(Value::as_str) {
                session = Some(id.to_string());
            }
        }
        if kind == Some("text") {
            if let Some(text) = event.pointer("/part/text").and_then(Value::as_str) {
                reply.push_str(text);
            }
        }
    }
    let session =
        session.ok_or_else(|| "the OpenCode adapter returned no session identifier".to_string())?;
    Ok(OpenCodeRun { session, reply })
}

/// Reads a call's token usage from `opencode session export` output, or `None`
/// when the export cannot be read.
///
/// The usage is the sum of the session's assistant messages' reported tokens.
/// A session whose assistant messages carry no usage, and an export that is
/// unreadable or truncated, both report no usage, so the prompt is excluded
/// from the run's median rather than counted as zero or failing the call
/// (FEAT-023, D-052).
pub fn parse_opencode_usage(text: &str) -> Option<u64> {
    let value: Value = serde_json::from_str(text).ok()?;
    let messages = value.get("messages").and_then(Value::as_array)?;
    let mut reported: Option<u64> = None;
    for message in messages {
        if message.get("type").and_then(Value::as_str) != Some("assistant") {
            continue;
        }
        if let Some(total) = message.get("tokens").and_then(total_usage) {
            reported = Some(reported.unwrap_or(0) + total);
        }
    }
    reported
}

/// The token count an OpenCode usage object reports: the input, output and
/// reasoning tokens the provider bills. A cache read is a subset of the input
/// and is not added again.
fn total_usage(tokens: &Value) -> Option<u64> {
    let count = |key: &str| tokens.get(key).and_then(Value::as_u64);
    let (input, output, reasoning) = (count("input"), count("output"), count("reasoning"));
    if input.is_none() && output.is_none() && reasoning.is_none() {
        return None;
    }
    Some(input.unwrap_or(0) + output.unwrap_or(0) + reasoning.unwrap_or(0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::corpus::Prompt;

    fn prompt(id: &str) -> Prompt {
        Prompt {
            id: id.to_string(),
            corpus_id: "c".to_string(),
            intent: "a red circle".to_string(),
            text: "draw a red circle".to_string(),
            order: 0,
            complexity: crate::corpus::Complexity::Simple,
            qualities: Vec::new(),
        }
    }

    #[test]
    fn parses_a_provider_model_reference() {
        let spec = parse_model_spec("openai:gpt-4o-2024-11-20").expect("parses");
        assert_eq!(spec.provider, ProviderKind::OpenAi);
        assert_eq!(spec.model, "gpt-4o-2024-11-20");
        assert_eq!(spec.id(), "openai:gpt-4o-2024-11-20");
    }

    #[test]
    fn a_replay_model_keeps_its_directory() {
        let spec = parse_model_spec("replay:/tmp/recorded").expect("parses");
        assert_eq!(spec.provider, ProviderKind::Replay);
        assert_eq!(spec.model, "/tmp/recorded");
    }

    #[test]
    fn an_unknown_provider_or_missing_model_is_refused() {
        assert!(parse_model_spec("nope:model").is_err());
        assert!(parse_model_spec("openai").is_err());
        assert!(parse_model_spec("openai:").is_err());
    }

    #[test]
    fn builds_a_vendor_request_body() {
        let openai = build_body(ProviderKind::OpenAi, "gpt", "sys", "usr");
        assert_eq!(openai["temperature"], 0);
        assert_eq!(openai["messages"][0]["role"], "system");
        assert_eq!(openai["messages"][1]["content"], "usr");

        let anthropic = build_body(ProviderKind::Anthropic, "claude", "sys", "usr");
        assert_eq!(anthropic["system"], "sys");
        assert_eq!(anthropic["messages"][0]["content"], "usr");

        let google = build_body(ProviderKind::Google, "gemini", "sys", "usr");
        assert_eq!(google["systemInstruction"]["parts"][0]["text"], "sys");
        assert_eq!(google["generationConfig"]["temperature"], 0);
    }

    #[test]
    fn reads_each_vendor_reply() {
        let openai = parse_body(
            ProviderKind::OpenAi,
            &json!({"choices": [{"message": {"content": "{\"a\":1}"}}], "usage": {"total_tokens": 7}}),
        )
        .expect("reads");
        assert_eq!(openai.text, "{\"a\":1}");
        assert_eq!(openai.tokens, Some(7));

        let anthropic = parse_body(
            ProviderKind::Anthropic,
            &json!({"content": [{"text": "hi"}], "usage": {"input_tokens": 2, "output_tokens": 3}}),
        )
        .expect("reads");
        assert_eq!(anthropic.tokens, Some(5));

        let google = parse_body(
            ProviderKind::Google,
            &json!({"candidates": [{"content": {"parts": [{"text": "hi"}]}}], "usageMetadata": {"totalTokenCount": 9}}),
        )
        .expect("reads");
        assert_eq!(google.tokens, Some(9));

        assert!(parse_body(ProviderKind::OpenAi, &json!({"choices": []})).is_err());
    }

    #[test]
    fn parses_a_judge_verdict() {
        let judgement = parse_judgement(
            r#"{"fidelity": 0.75, "qualities": {"depth": true, "subject_accuracy": true}}"#,
        )
        .expect("parses");
        assert_eq!(judgement.fidelity, 0.75);
        assert!(judgement.qualities["depth"]);
        assert!(!judgement.qualities["shared_anchors"]);
    }

    #[test]
    fn a_judge_verdict_is_clamped_and_a_missing_number_is_refused() {
        let judgement = parse_judgement(r#"{"fidelity": 2.0, "qualities": {}}"#).expect("parses");
        assert_eq!(judgement.fidelity, 1.0);
        assert!(parse_judgement(r#"{"qualities": {}}"#).is_err());
    }

    #[test]
    fn the_authoring_system_prompt_carries_the_schema_the_skill_and_the_tokens() {
        let system = authoring_system();
        // The published schema, the request contract and the palette tokens.
        assert!(system.contains("Vectr"), "names the tool");
        assert!(system.contains("palette tokens"), "lists the tokens");
        assert!(system.contains("formatVersion"), "carries the schema");
        // The skill's instructions: the always-read entry point (FEAT-020).
        assert!(
            system.contains("## Version compatibility"),
            "carries the skill entry point"
        );
        assert!(
            system.contains("references/rules.md"),
            "carries the entry point's pointer to its references"
        );
        // The rules the models most often violate, stated in the versioned
        // authoring prompt where a one-shot call sees them: the flat element
        // list, the single owner, the unique accessible name, and the style
        // assets it may reference.
        for rule in ["parentId", "sceneId", "definitionId", "accessibleName"] {
            assert!(system.contains(rule), "states the `{rule}` rule");
        }
        assert!(
            system.contains("stroke profiles"),
            "names the available stroke profiles"
        );
    }

    #[test]
    fn replay_reads_a_recorded_authoring_output_and_judgement() {
        let dir = crate::testing::TempDir::new("replay");
        std::fs::write(
            dir.path().join("p1.scene.json"),
            r#"{"scene": {"id": "s"}}"#,
        )
        .expect("writes");
        std::fs::write(
            dir.path().join("p1.judge.json"),
            r#"{"fidelity": 0.5, "qualities": {"depth": true}}"#,
        )
        .expect("writes");

        let spec = ModelSpec {
            provider: ProviderKind::Replay,
            model: dir.path().display().to_string(),
        };
        let (author, judge) = build(&spec).expect("builds");
        let authored = author.author(&prompt("p1")).expect("authors");
        assert!(authored.text.contains("\"scene\""));
        let judgement = judge.judge(&prompt("p1"), &authored).expect("judges");
        assert_eq!(judgement.fidelity, 0.5);
        assert!(judgement.qualities["depth"]);
    }

    #[test]
    fn replay_names_a_missing_recording() {
        let dir = crate::testing::TempDir::new("replay-missing");
        let spec = ModelSpec {
            provider: ProviderKind::Replay,
            model: dir.path().display().to_string(),
        };
        let (author, _) = build(&spec).expect("builds");
        let error = author.author(&prompt("absent")).expect_err("refused");
        assert!(error.0.contains("absent"), "{}", error.0);
    }

    #[test]
    fn replay_reads_a_recorded_usage_and_treats_a_missing_one_as_unreported() {
        let dir = crate::testing::TempDir::new("replay-usage");
        std::fs::write(
            dir.path().join("p1.scene.json"),
            r#"{"scene": {"id": "s"}}"#,
        )
        .expect("writes");
        let spec = ModelSpec {
            provider: ProviderKind::Replay,
            model: dir.path().display().to_string(),
        };
        let (author, _) = build(&spec).expect("builds");

        assert_eq!(author.author(&prompt("p1")).expect("authors").tokens, None);
        crate::testing::write_usage(dir.path(), "p1", 12_345);
        assert_eq!(
            author.author(&prompt("p1")).expect("authors").tokens,
            Some(12_345)
        );
    }

    #[test]
    fn replay_refuses_a_malformed_usage_recording() {
        let dir = crate::testing::TempDir::new("replay-usage-bad");
        std::fs::write(
            dir.path().join("p1.scene.json"),
            r#"{"scene": {"id": "s"}}"#,
        )
        .expect("writes");
        std::fs::write(dir.path().join("p1.usage.json"), r#"{"cost": 3}"#).expect("writes");
        let spec = ModelSpec {
            provider: ProviderKind::Replay,
            model: dir.path().display().to_string(),
        };
        let (author, _) = build(&spec).expect("builds");
        let error = author.author(&prompt("p1")).expect_err("refused");
        assert!(error.0.contains("tokens"), "{}", error.0);
    }

    #[test]
    fn the_opencode_run_arguments_select_the_tool_free_agent_and_json_stream() {
        let args = opencode_run_args("opencode-go/deepseek-v4-flash", "draw a dot");
        assert_eq!(args[0], "run");
        // `--format json` is what surfaces the session identifier.
        let format = args
            .iter()
            .position(|arg| arg == "--format")
            .expect("format");
        assert_eq!(args[format + 1], "json");
        let agent = args.iter().position(|arg| arg == "--agent").expect("agent");
        assert_eq!(args[agent + 1], OPENCODE_AGENT);
        let model = args.iter().position(|arg| arg == "--model").expect("model");
        assert_eq!(args[model + 1], "opencode-go/deepseek-v4-flash");
        // `--` keeps a leading dash in the message from reading as a flag.
        let dash = args.iter().rposition(|arg| arg == "--").expect("separator");
        assert_eq!(args[dash + 1], "draw a dot");
    }

    #[test]
    fn the_opencode_config_declares_a_tool_free_agent_carrying_the_system() {
        let value: Value =
            serde_json::from_str(&opencode_config("You author scenes.")).expect("valid JSON");
        let agent = &value["agents"][OPENCODE_AGENT];
        assert_eq!(agent["system"], "You author scenes.");
        assert_eq!(agent["mode"], "primary");
        assert_eq!(agent["permissions"][0]["action"], "*");
        assert_eq!(agent["permissions"][0]["effect"], "deny");
    }

    #[test]
    fn reads_the_session_and_streamed_reply_from_the_opencode_event_stream() {
        let stream = [
            json!({ "type": "step_start", "sessionID": "ses_abc" }),
            json!({ "type": "reasoning", "sessionID": "ses_abc", "part": { "type": "reasoning", "text": "thinking" } }),
            json!({ "type": "text", "sessionID": "ses_abc", "part": { "type": "text", "text": "{\"scene\"" } }),
            json!({ "type": "text", "sessionID": "ses_abc", "part": { "type": "text", "text": ": {\"id\":\"s\"}}" } }),
        ]
        .into_iter()
        .map(|event| event.to_string())
        .collect::<Vec<_>>()
        .join("\n");
        let run = parse_opencode_run(&stream).expect("reads");
        assert_eq!(run.session, "ses_abc");
        // The reply is the streamed text parts, joined; reasoning is not text.
        assert_eq!(run.reply, "{\"scene\": {\"id\":\"s\"}}");
    }

    #[test]
    fn an_opencode_error_event_or_a_missing_session_is_refused() {
        let error = parse_opencode_run(
            "{\"type\":\"error\",\"error\":{\"message\":\"model not found\"}}\n",
        )
        .expect_err("refused");
        assert!(error.contains("model not found"), "{error}");
        assert!(parse_opencode_run("not json\n").is_err());
    }

    #[test]
    fn reads_an_opencode_sessions_reported_usage() {
        let export = json!({
            "info": { "tokens": { "input": 9999, "output": 9999, "reasoning": 0, "cache": {} } },
            "messages": [
                { "type": "user", "text": "draw a dot" },
                { "type": "assistant", "content": [
                    { "type": "text", "text": "{}" }
                ], "tokens": { "input": 200, "output": 3, "reasoning": 5, "cache": { "read": 40, "write": 0 } } },
                { "type": "assistant", "content": [
                    { "type": "text", "text": "{}" }
                ], "tokens": { "input": 100, "output": 2, "reasoning": 0, "cache": { "read": 0, "write": 0 } } }
            ]
        });
        // The session aggregate (9999) is not the call's usage; only the
        // assistant messages are. Cache reads are a subset of the input.
        assert_eq!(
            parse_opencode_usage(&export.to_string()),
            Some(200 + 3 + 5 + 100 + 2)
        );
    }

    #[test]
    fn an_opencode_session_that_reports_no_usage_reports_none() {
        let export = json!({
            "messages": [
                { "type": "assistant", "content": [{ "type": "text", "text": "{}" }] }
            ]
        });
        assert_eq!(parse_opencode_usage(&export.to_string()), None);
    }

    #[test]
    fn an_unreadable_or_truncated_usage_export_reports_no_usage() {
        // A truncated export is not valid JSON: no usage, not a failed call.
        assert_eq!(
            parse_opencode_usage("{\"messages\": [{\"type\":\"assist"),
            None
        );
        assert_eq!(parse_opencode_usage("not json"), None);
        // A well-formed export with no messages carries no usage either.
        assert_eq!(parse_opencode_usage("{}"), None);
    }

    #[test]
    fn a_bounded_retry_absorbs_a_transient_failure() {
        let mut attempts = 0;
        let value = retrying(3, Duration::ZERO, |attempt| {
            attempts += 1;
            if attempt < 3 {
                Err(ProviderError(format!("transient {attempt}")))
            } else {
                Ok(attempt)
            }
        })
        .expect("succeeds on the third attempt");
        assert_eq!(value, 3);
        assert_eq!(attempts, 3);
    }

    #[test]
    fn a_bounded_retry_stops_after_its_bound() {
        let mut attempts = 0;
        let error = retrying(3, Duration::ZERO, |_attempt| {
            attempts += 1;
            Err::<u8, _>(ProviderError("still failing".to_string()))
        })
        .expect_err("refused");
        assert_eq!(attempts, 3, "the retry is bounded");
        assert_eq!(error.0, "still failing");
    }

    #[test]
    fn the_retry_backoff_grows_with_each_attempt() {
        let base = Duration::from_millis(500);
        assert_eq!(retry_delay(base, 1), Duration::from_millis(500));
        assert_eq!(retry_delay(base, 2), Duration::from_millis(1_000));
        assert_eq!(retry_delay(base, 3), Duration::from_millis(2_000));
    }

    /// A completion that fails its first `failures` calls and then succeeds, so
    /// a retry's attempts can be counted.
    struct Flaky {
        failures: usize,
        calls: std::sync::Arc<AtomicUsize>,
    }

    impl Completion for Flaky {
        fn complete(&self, _system: &str, _user: &str) -> Result<Authored, ProviderError> {
            let call = self.calls.fetch_add(1, Ordering::Relaxed) + 1;
            if call <= self.failures {
                Err(ProviderError(format!("transient {call}")))
            } else {
                Ok(Authored {
                    text: "{}".to_string(),
                    tokens: None,
                })
            }
        }
    }

    fn retrying_client(failures: usize, calls: &std::sync::Arc<AtomicUsize>) -> RetryingClient {
        RetryingClient {
            inner: Box::new(Flaky {
                failures,
                calls: calls.clone(),
            }),
            attempts: RETRY_ATTEMPTS,
            backoff: Duration::ZERO,
        }
    }

    #[test]
    fn a_provider_call_is_retried_before_it_gives_up() {
        let calls = std::sync::Arc::new(AtomicUsize::new(0));
        let client = retrying_client(1, &calls);
        let authored = client.complete("sys", "usr").expect("the retry succeeds");
        assert_eq!(authored.text, "{}");
        assert_eq!(
            calls.load(Ordering::Relaxed),
            2,
            "the transient provider failure was retried"
        );
    }

    #[test]
    fn a_provider_call_that_keeps_failing_gives_up_at_the_bound() {
        let calls = std::sync::Arc::new(AtomicUsize::new(0));
        let client = retrying_client(usize::MAX, &calls);
        client.complete("sys", "usr").expect_err("refused");
        assert_eq!(
            calls.load(Ordering::Relaxed),
            RETRY_ATTEMPTS,
            "the provider retry is bounded"
        );
    }

    #[test]
    fn the_wrapped_client_uses_the_harness_retry_settings() {
        let client = RetryingClient::new(Box::new(Flaky {
            failures: 0,
            calls: std::sync::Arc::new(AtomicUsize::new(0)),
        }));
        assert_eq!(client.attempts, RETRY_ATTEMPTS);
        assert_eq!(client.backoff, RETRY_BACKOFF);
    }

    #[test]
    fn reads_recorded_artifacts_for_a_resume_and_reports_a_missing_one() {
        let dir = crate::testing::TempDir::new("recorded");
        assert!(read_recorded_authored(dir.path(), "p1")
            .expect("reads")
            .is_none());
        assert!(read_recorded_judgement(dir.path(), "p1")
            .expect("reads")
            .is_none());

        std::fs::write(dir.path().join("p1.scene.json"), r#"{"scene":{}}"#).expect("writes");
        std::fs::write(
            dir.path().join("p1.judge.json"),
            r#"{"fidelity":0.4,"qualities":{"depth":true}}"#,
        )
        .expect("writes");
        let authored = read_recorded_authored(dir.path(), "p1")
            .expect("reads")
            .expect("the reply is recorded");
        assert!(authored.text.contains("scene"), "{}", authored.text);
        assert_eq!(authored.tokens, None);
        let judgement = read_recorded_judgement(dir.path(), "p1")
            .expect("reads")
            .expect("the verdict is recorded");
        assert_eq!(judgement.fidelity, 0.4);
        assert!(judgement.qualities["depth"]);
    }

    #[test]
    fn a_recorded_usage_is_carried_into_a_resumed_authoring_reply() {
        let dir = crate::testing::TempDir::new("recorded-usage");
        std::fs::write(dir.path().join("p1.scene.json"), r#"{"scene":{}}"#).expect("writes");
        crate::testing::write_usage(dir.path(), "p1", 7_000);
        let authored = read_recorded_authored(dir.path(), "p1")
            .expect("reads")
            .expect("the reply is recorded");
        assert_eq!(authored.tokens, Some(7_000));
    }

    #[test]
    fn a_recorded_verdict_that_is_not_a_verdict_is_left_for_the_judge() {
        let dir = crate::testing::TempDir::new("recorded-error");
        // The harness records a judge that returned no verdict as an error
        // document; on resume that is no verdict, so the judge is asked again.
        std::fs::write(
            dir.path().join("p1.judge.json"),
            r#"{"error":"the judge returned no verdict"}"#,
        )
        .expect("writes");
        assert!(read_recorded_judgement(dir.path(), "p1")
            .expect("reads")
            .is_none());
    }
}

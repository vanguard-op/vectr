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

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{json, Value};

use crate::corpus::{Prompt, Quality};
use crate::json::extract_json;
use crate::toolchain::PALETTE_TOKENS;

/// The versioned authoring prompt artifact: the system prompt the harness
/// sends with the published schema and the packaged guidance.
const AUTHORING_PROMPT: &str = include_str!("../prompts/authoring.v1.txt");

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
            client: author_client,
        }),
        Box::new(LlmJudge {
            client: judge_client,
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
/// schema, the packaged guidance and the palette tokens the evaluation project
/// supplies (FEAT-032 Model Behavior).
pub fn authoring_system() -> String {
    let schema = vectr_core::schema(vectr_core::SchemaForm::Compact).unwrap_or_default();
    let guide = vectr_project::authoring_guide();
    let tokens: Vec<String> = PALETTE_TOKENS
        .iter()
        .map(|(name, value)| format!("- {name}: {value}"))
        .collect();
    format!(
        "{AUTHORING_PROMPT}\n\n# Published schema (compact)\n\n{schema}\n\n# Authoring guide\n\n{guide}\n\n# Available palette tokens\n\n{}\n",
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
struct OpenCodeModel {
    model: String,
    bin: String,
}

impl Completion for OpenCodeModel {
    fn complete(&self, system: &str, user: &str) -> Result<Authored, ProviderError> {
        let combined = format!("{system}\n\n{user}");
        let output = Command::new(&self.bin)
            .args(["run", "--model", &self.model, &combined])
            .output()
            .map_err(|error| {
                ProviderError(format!(
                    "the OpenCode adapter could not run `{}`: {error}",
                    self.bin
                ))
            })?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(ProviderError(format!(
                "the OpenCode adapter failed for model `{}`: {}",
                self.model,
                stderr.trim()
            )));
        }
        Ok(Authored {
            text: String::from_utf8_lossy(&output.stdout).into_owned(),
            tokens: None,
        })
    }
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
    fn the_authoring_system_prompt_carries_the_guidance_and_tokens() {
        let system = authoring_system();
        assert!(system.contains("Vectr"), "names the tool");
        assert!(system.contains("palette tokens"), "lists the tokens");
        assert!(system.contains("vectr validate"), "carries the guide");
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
}

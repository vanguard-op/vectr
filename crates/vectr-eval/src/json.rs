//! Pulling one JSON object out of a model's reply.
//!
//! A model is asked to return JSON and nothing else, but a reply may still be
//! wrapped in prose or a fenced code block. The extractor finds the first
//! balanced JSON object or array in the text, ignoring braces that appear
//! inside strings, so a recorded reply is read the same way a live one is
//! (FEAT-023).

use serde_json::Value;

/// Extracts the first balanced JSON object or array from `text`.
///
/// Returns the parsed value, or `None` when the text holds no complete JSON
/// value. A fenced ```json block is unwrapped first so a reply that fences its
/// JSON is read as JSON rather than prose.
pub fn extract_json(text: &str) -> Option<Value> {
    let candidate = unwrap_fence(text).unwrap_or_else(|| text.to_string());
    let start = candidate
        .char_indices()
        .find(|(_, ch)| *ch == '{' || *ch == '[')
        .map(|(index, _)| index)?;
    let open = candidate.as_bytes()[start] as char;
    let close = if open == '{' { '}' } else { ']' };
    let end = balanced_end(&candidate, start, open, close)?;
    serde_json::from_str(&candidate[start..=end]).ok()
}

/// The text inside the first fenced code block, when the reply carries one.
fn unwrap_fence(text: &str) -> Option<String> {
    let fence = text.find("```")?;
    let after = &text[fence + 3..];
    // Skip an optional language tag up to the end of the line.
    let body_start = after.find('\n').map(|index| index + 1).unwrap_or(0);
    let body = &after[body_start..];
    let end = body.find("```")?;
    Some(body[..end].to_string())
}

/// The index of the `close` delimiter that balances the `open` at `start`,
/// skipping delimiters inside JSON strings.
fn balanced_end(text: &str, start: usize, open: char, close: char) -> Option<usize> {
    let mut depth = 0i32;
    let mut in_string = false;
    let mut escaped = false;
    for (offset, ch) in text[start..].char_indices() {
        if in_string {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }
        match ch {
            '"' => in_string = true,
            c if c == open => depth += 1,
            c if c == close => {
                depth -= 1;
                if depth == 0 {
                    return Some(start + offset);
                }
            }
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn extracts_a_bare_object() {
        assert_eq!(extract_json(r#"{"a":1}"#), Some(json!({"a": 1})));
    }

    #[test]
    fn extracts_through_prose_and_a_fence() {
        let reply = "Here is the scene:\n```json\n{\"a\": 1}\n```\nEnjoy.";
        assert_eq!(extract_json(reply), Some(json!({"a": 1})));
    }

    #[test]
    fn ignores_braces_inside_strings() {
        let reply = r#"prefix {"text":"a } brace","n":2} suffix"#;
        assert_eq!(
            extract_json(reply),
            Some(json!({"text": "a } brace", "n": 2}))
        );
    }

    #[test]
    fn an_incomplete_object_is_not_extracted() {
        assert_eq!(extract_json("{\"a\": 1"), None);
        assert_eq!(extract_json("no json here"), None);
    }
}

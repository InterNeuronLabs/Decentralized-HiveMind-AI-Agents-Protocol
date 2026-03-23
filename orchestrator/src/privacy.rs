// orchestrator/src/privacy.rs
// PII tokenisation for Tier 2/3 jobs.
// Replaces sensitive tokens with UUID placeholders before dispatch to nodes.
// The mapping exists ONLY in memory for the job lifetime — never written to DB or logs.

use common::types::PiiMap;
use once_cell::sync::Lazy;
use regex::Regex;
use uuid::Uuid;

/// Patterns considered sensitive and eligible for replacement.
/// Order matters: more specific patterns first.
static PII_PATTERNS: Lazy<Vec<(&'static str, Regex)>> = Lazy::new(|| {
    vec![
        // API keys / secrets (common prefixes)
        (
            "api_key",
            Regex::new(r"(?i)(sk-|api[_-]?key[_-]?=?)\s*([A-Za-z0-9\-_]{20,})").unwrap(),
        ),
        // Email addresses
        (
            "email",
            Regex::new(r"[a-zA-Z0-9._%+\-]+@[a-zA-Z0-9.\-]+\.[a-zA-Z]{2,}").unwrap(),
        ),
        // Phone numbers (E.164 and common US formats)
        (
            "phone",
            Regex::new(r"\+?[0-9]{1,3}[\s\-]?\(?\d{3}\)?[\s\-]?\d{3}[\s\-]?\d{4}").unwrap(),
        ),
        // Credit card numbers (basic 13-19 digit runs)
        ("cc", Regex::new(r"\b[0-9]{13,19}\b").unwrap()),
        // IPv4 addresses
        ("ip", Regex::new(r"\b(?:\d{1,3}\.){3}\d{1,3}\b").unwrap()),
    ]
});

/// Tokenise sensitive values in `text`, returning the sanitised text and the
/// PiiMap needed to de-tokenise the final aggregated output.
pub fn tokenise(text: &str) -> (String, PiiMap) {
    let mut result = text.to_owned();
    let mut map = PiiMap::new();

    for (label, pattern) in PII_PATTERNS.iter() {
        let replaced = pattern.replace_all(&result, |caps: &regex::Captures| {
            let original = caps[0].to_string();
            let placeholder = format!("[PII_{label}_{id}]", id = &Uuid::new_v4().to_string()[..8]);
            map.insert(placeholder.clone(), original);
            placeholder
        });
        result = replaced.into_owned();
    }

    (result, map)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn email_is_replaced() {
        let (out, map) = tokenise("Contact me at alice@example.com please");
        assert!(!out.contains("alice@example.com"));
        assert!(out.contains("[PII_email_"));
        // De-tokenising restores the original
        let restored = map.detokenize(&out);
        assert!(restored.contains("alice@example.com"));
    }

    #[test]
    fn no_pii_unchanged() {
        let text = "Hello world, this is fine.";
        let (out, _) = tokenise(text);
        assert_eq!(out, text);
    }

    #[test]
    fn api_key_is_replaced() {
        let (out, _) = tokenise("Use sk-abc123XYZ789longkeyhere for the API");
        assert!(!out.contains("sk-abc123XYZ789longkeyhere"));
    }
}

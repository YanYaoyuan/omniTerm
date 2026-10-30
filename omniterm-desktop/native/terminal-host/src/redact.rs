//! Best-effort removal of credentials before terminal text leaves the machine. It cannot
//! recognize every secret, which is why the UI still shows what is shared and allows pausing.

use regex_lite::Regex;
use std::sync::OnceLock;

const MASK: &str = "[REDACTED]";

fn patterns() -> &'static [(Regex, &'static str)] {
    static PATTERNS: OnceLock<Vec<(Regex, &'static str)>> = OnceLock::new();
    PATTERNS.get_or_init(|| {
        [
            (r"(?i)\b(password|passwd|pwd|secret|token|api[_-]?key|access[_-]?key)(\s*[:=]\s*)\S+", "$1$2[REDACTED]"),
            (r"(?i)\b(authorization:\s*(?:bearer|basic)\s+)\S+", "$1[REDACTED]"),
            (r"\b(sk|rk|pk)-[A-Za-z0-9_-]{16,}", MASK),
            (r"\bgh[pousr]_[A-Za-z0-9]{20,}", MASK),
            (r"\bgithub_pat_[A-Za-z0-9_]{20,}", MASK),
            (r"\bAKIA[0-9A-Z]{16}\b", MASK),
            (r"\bxox[abprs]-[A-Za-z0-9-]{10,}", MASK),
            (r"\beyJ[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}", MASK),
            (r"-----BEGIN [A-Z ]*PRIVATE KEY-----", "[REDACTED PRIVATE KEY]"),
        ]
        .into_iter()
        .map(|(pattern, replacement)| {
            (Regex::new(pattern).expect("redaction pattern is valid"), replacement)
        })
        .collect()
    })
}

pub fn redact(line: &str) -> String {
    let mut text = line.to_string();
    for (pattern, replacement) in patterns() {
        if pattern.is_match(&text) {
            text = pattern.replace_all(&text, *replacement).into_owned();
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use super::redact;

    #[test]
    fn known_credential_shapes_are_masked() {
        assert_eq!(redact("export API_KEY=abc123 ok"), "export API_KEY=[REDACTED] ok");
        assert_eq!(redact("Authorization: Bearer abc.def"), "Authorization: Bearer [REDACTED]");
        assert_eq!(redact("key sk-abcdefghijklmnop1234"), "key [REDACTED]");
        assert_eq!(redact("AKIAABCDEFGHIJKLMNOP"), "[REDACTED]");
        assert_eq!(redact("-----BEGIN OPENSSH PRIVATE KEY-----"), "[REDACTED PRIVATE KEY]");
    }

    #[test]
    fn ordinary_output_is_untouched() {
        for line in ["build failed: 3 errors", "tokens used: many", "skip-this-step"] {
            assert_eq!(redact(line), line);
        }
    }
}

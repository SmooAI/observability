//! PII scrubbing — applied to message strings, breadcrumb messages, and headers
//! before transport. Stays opinionated and minimal; tenants can extend in
//! `before_send`.
//!
//! Direct port of the TS `pii.ts` patterns so Rust + TS scrub identically.

use once_cell::sync::Lazy;
use regex::Regex;
use std::collections::BTreeMap;

struct PiiPattern {
    re: Regex,
    /// `None` means "use a closure-style key-preserving replacement" (the
    /// token/api-key/secret pattern in the TS SDK replaces only the value after
    /// the `=`/`:`); `Some(s)` is a fixed replacement string.
    replacement: Option<&'static str>,
}

static PII_PATTERNS: Lazy<Vec<PiiPattern>> = Lazy::new(|| {
    vec![
        // Bearer tokens.
        PiiPattern {
            re: Regex::new(r"(?i)Bearer\s+[A-Za-z0-9._-]+").unwrap(),
            replacement: Some("Bearer [redacted]"),
        },
        // password / passwd / pwd = value
        PiiPattern {
            re: Regex::new(r#"(?i)\b(?:password|passwd|pwd)["']?\s*[:=]\s*["']?[^"'&\s]+"#)
                .unwrap(),
            replacement: Some("password=[redacted]"),
        },
        // token / api-key / apikey / secret = value — preserve the key, redact
        // only the value (matches the TS closure replacement behavior).
        PiiPattern {
            re: Regex::new(
                r#"(?i)\b(?:token|api[-_]?key|apikey|secret)["']?\s*[:=]\s*["']?[^"'&\s]+"#,
            )
            .unwrap(),
            replacement: None,
        },
        // OpenAI-style sk- keys.
        PiiPattern {
            re: Regex::new(r"sk-[A-Za-z0-9]{20,}").unwrap(),
            replacement: Some("sk-[redacted]"),
        },
    ]
});

static SECRET_KEY_VALUE_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"^(.*?[:=]).*$").unwrap());

const SENSITIVE_HEADERS: &[&str] = &[
    "authorization",
    "cookie",
    "set-cookie",
    "x-api-key",
    "x-auth-token",
];

/// Scrub a free-form string. Idempotent enough for repeated calls.
pub fn scrub_string(input: &str) -> String {
    let mut out = input.to_string();
    for pattern in PII_PATTERNS.iter() {
        out = match pattern.replacement {
            Some(repl) => pattern.re.replace_all(&out, repl).into_owned(),
            None => {
                // Key-preserving: keep everything up to and including the
                // `=`/`:` separator, redact the rest. Mirrors the TS
                // `'$&'.replace(/=.*/, '=[redacted]')` intent.
                pattern
                    .re
                    .replace_all(&out, |caps: &regex::Captures| {
                        let matched = &caps[0];
                        SECRET_KEY_VALUE_RE
                            .replace(matched, "$1[redacted]")
                            .into_owned()
                    })
                    .into_owned()
            }
        };
    }
    out
}

/// Scrub a header map: sensitive header names are fully redacted, all other
/// values are run through [`scrub_string`]. Header-name comparison is
/// case-insensitive.
pub fn scrub_headers(headers: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for (k, v) in headers {
        if SENSITIVE_HEADERS.contains(&k.to_lowercase().as_str()) {
            out.insert(k.clone(), "[redacted]".to_string());
        } else {
            out.insert(k.clone(), scrub_string(v));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scrubs_bearer_tokens() {
        let s = scrub_string("Authorization: Bearer abc.def-ghi_123");
        assert!(s.contains("Bearer [redacted]"), "{s}");
        assert!(!s.contains("abc.def"));
    }

    #[test]
    fn scrubs_passwords() {
        let s = scrub_string("login password=hunter2 ok");
        assert!(s.contains("password=[redacted]"), "{s}");
        assert!(!s.contains("hunter2"));
    }

    #[test]
    fn scrubs_token_preserving_key() {
        let s = scrub_string("api_key=supersecretvalue");
        assert!(s.starts_with("api_key="), "{s}");
        assert!(s.contains("[redacted]"), "{s}");
        assert!(!s.contains("supersecretvalue"), "{s}");
    }

    #[test]
    fn scrubs_sk_keys() {
        let s = scrub_string("key sk-ABCDEFGHIJKLMNOPQRSTUVWX123");
        assert!(s.contains("sk-[redacted]"), "{s}");
    }

    #[test]
    fn redacts_sensitive_headers() {
        let mut h = BTreeMap::new();
        h.insert("Authorization".to_string(), "Bearer xyz".to_string());
        h.insert("Cookie".to_string(), "session=abc".to_string());
        h.insert("Content-Type".to_string(), "application/json".to_string());
        let out = scrub_headers(&h);
        assert_eq!(out["Authorization"], "[redacted]");
        assert_eq!(out["Cookie"], "[redacted]");
        assert_eq!(out["Content-Type"], "application/json");
    }

    #[test]
    fn non_sensitive_header_values_are_scrubbed() {
        let mut h = BTreeMap::new();
        h.insert(
            "X-Debug".to_string(),
            "tried Bearer leakedtoken123".to_string(),
        );
        let out = scrub_headers(&h);
        assert!(
            out["X-Debug"].contains("Bearer [redacted]"),
            "{}",
            out["X-Debug"]
        );
    }

    #[test]
    fn clean_string_unchanged() {
        assert_eq!(
            scrub_string("nothing sensitive here"),
            "nothing sensitive here"
        );
    }
}

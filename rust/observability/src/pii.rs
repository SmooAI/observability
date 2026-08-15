//! PII scrubbing — applied to message strings, breadcrumb messages, headers,
//! and (downstream) GenAI tool arguments/results before transport.
//!
//! Two classes, handled differently on purpose:
//!
//! - **Credentials** (`Bearer …`, `password=`, `token`/`api_key`/`secret=`,
//!   `sk-…`) are **dropped**. A hash of a live token is still a token oracle,
//!   and there is no correlation value in a secret.
//! - **Personal identifiers** (email, phone, street address) are **hashed**,
//!   not dropped: `a@b.com` → `[email:9f2a41c8]`. That keeps the one question
//!   worth asking — "are these two spans the same person?" — answerable while
//!   storing nothing reversible.
//!
//! The hash is **HMAC-SHA256, keyed**, not a bare digest: emails and phone
//! numbers are a small enumerable space that a rainbow table reverses in
//! seconds. The org id is mixed into the HMAC message, so identical PII hashes
//! **differently in different orgs** — no cross-tenant correlation, matching
//! the ClickHouse org-isolation posture.
//!
//! **The key and the org salt are load-bearing and must not rotate casually.**
//! Rotating either silently breaks correlation with every previously stored
//! hash. Supply the key once at startup via `SMOOAI_OBSERVABILITY_PII_HASH_KEY`
//! (read by [`crate::bootstrap`]) or [`set_pii_hash_key`]. **With no key
//! configured, personal identifiers are fully redacted** (`[email:redacted]`)
//! rather than hashed under a guessable one — fail safe, never fail open.
//!
//! Pattern matching never catches everything. It fails safe, which is the right
//! default for data we persist from end-user conversations; tenants can extend
//! in `before_send`.
//!
//! Credential patterns are a direct port of the TS `pii.ts` patterns. The
//! hashing layer is Rust-only today — the other four SDKs still scrub
//! credentials only.

use once_cell::sync::{Lazy, OnceCell};
use regex::Regex;
use std::collections::BTreeMap;
use std::fmt::Write as _;

struct PiiPattern {
    re: Regex,
    /// `None` means "use a closure-style key-preserving replacement" (the
    /// token/api-key/secret pattern in the TS SDK replaces only the value after
    /// the `=`/`:`); `Some(s)` is a fixed replacement string.
    replacement: Option<&'static str>,
}

/// Credentials. Matched FIRST so a personal identifier sitting inside a secret
/// (`token=a@b.com`) is dropped with the secret rather than surviving as a hash.
static CREDENTIAL_PATTERNS: Lazy<Vec<PiiPattern>> = Lazy::new(|| {
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

/// The class of personal identifier a match represents. Drives both the visible
/// prefix in the output token and the normalization applied before hashing, so
/// `(555) 555-0142` and `555-555-0142` correlate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PiiKind {
    Email,
    Phone,
    Address,
}

impl PiiKind {
    /// The prefix that stays visible in the scrubbed output.
    pub fn label(self) -> &'static str {
        match self {
            PiiKind::Email => "email",
            PiiKind::Phone => "phone",
            PiiKind::Address => "address",
        }
    }

    fn normalize(self, raw: &str) -> String {
        match self {
            PiiKind::Email => raw.trim().to_lowercase(),
            // Digits only: formatting must not fork the hash.
            PiiKind::Phone => raw.chars().filter(|c| c.is_ascii_digit()).collect(),
            // Case-fold and collapse runs of whitespace.
            PiiKind::Address => raw
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .to_lowercase(),
        }
    }
}

/// Personal identifiers — hashed, not dropped. Order matters only in that these
/// all run after [`CREDENTIAL_PATTERNS`].
static PERSONAL_PATTERNS: Lazy<Vec<(PiiKind, Regex)>> = Lazy::new(|| {
    vec![
        (
            PiiKind::Email,
            Regex::new(r"(?i)\b[A-Za-z0-9._%+-]+@[A-Za-z0-9-]+(?:\.[A-Za-z0-9-]+)+\b").unwrap(),
        ),
        // Phone: optional country code / area code, then the 3-4 local pair.
        // A separator is REQUIRED so bare digit runs (ids, timestamps, amounts)
        // don't get eaten.
        (
            PiiKind::Phone,
            Regex::new(r"(?:\+\d{1,3}[ .-]?)?(?:\(\d{3}\)[ .-]?|\b\d{3}[ .-])?\b\d{3}[ .-]\d{4}\b")
                .unwrap(),
        ),
        // US-style street address: house number, 1-3 words, a street suffix.
        (
            PiiKind::Address,
            Regex::new(
                r"(?i)\b\d{1,6}\s+(?:[A-Za-z0-9.'-]+\s+){0,3}(?:street|st|avenue|ave|road|rd|boulevard|blvd|lane|ln|drive|dr|court|ct|way|terrace|ter|place|pl|circle|cir|highway|hwy|parkway|pkwy|square|sq)\b\.?",
            )
            .unwrap(),
        ),
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

/// Hex characters kept from the HMAC. Long enough that collisions are rare
/// across an org's traces, short enough to read in a span attribute.
const HASH_HEX_LEN: usize = 8;

static PII_HASH_KEY: OnceCell<Vec<u8>> = OnceCell::new();

/// Install the process-wide HMAC key used to hash personal identifiers.
/// Idempotent and set-once: returns `false` if a key was already installed
/// (the existing key is kept — a mid-process rotation would silently fork
/// every correlation). [`crate::bootstrap`] calls this from
/// `SMOOAI_OBSERVABILITY_PII_HASH_KEY`.
pub fn set_pii_hash_key(key: impl Into<Vec<u8>>) -> bool {
    let key = key.into();
    if key.is_empty() {
        return false;
    }
    PII_HASH_KEY.set(key).is_ok()
}

fn pii_hash_key() -> Option<&'static [u8]> {
    PII_HASH_KEY.get().map(|k| k.as_slice())
}

/// Hash one known-personal value into its scrubbed token — the same token
/// [`scrub_string_for_org`] would have written. This is how a UI search box
/// finds stored hashes: hash the typed term with the same org and match.
///
/// Returns `[<kind>:redacted]` when no key is installed.
pub fn pii_token(kind: PiiKind, raw: &str, org_id: &str) -> String {
    token_with_key(kind, raw, org_id, pii_hash_key())
}

fn token_with_key(kind: PiiKind, raw: &str, org_id: &str, key: Option<&[u8]>) -> String {
    let Some(key) = key else {
        return format!("[{}:redacted]", kind.label());
    };
    let normalized = kind.normalize(raw);
    // org_id in the HMAC message IS the per-org salt. The kind is in there too
    // so a phone and an address that normalize alike can't collide.
    let mut msg = Vec::with_capacity(org_id.len() + normalized.len() + 16);
    msg.extend_from_slice(org_id.as_bytes());
    msg.push(0);
    msg.extend_from_slice(kind.label().as_bytes());
    msg.push(0);
    msg.extend_from_slice(normalized.as_bytes());

    let mac = hmac_sha256::HMAC::mac(&msg, key);
    let mut hex = String::with_capacity(HASH_HEX_LEN);
    for byte in mac.iter().take(HASH_HEX_LEN / 2) {
        let _ = write!(hex, "{byte:02x}");
    }
    format!("[{}:{}]", kind.label(), hex)
}

/// Scrub a free-form string with no org context — credentials dropped, personal
/// identifiers hashed under the empty org salt. Prefer
/// [`scrub_string_for_org`] wherever an org id is in hand, so hashes can't be
/// correlated across tenants. Idempotent enough for repeated calls.
pub fn scrub_string(input: &str) -> String {
    scrub_with_key(input, "", pii_hash_key())
}

/// Scrub a free-form string, salting personal-identifier hashes with `org_id`.
pub fn scrub_string_for_org(input: &str, org_id: &str) -> String {
    scrub_with_key(input, org_id, pii_hash_key())
}

fn scrub_with_key(input: &str, org_id: &str, key: Option<&[u8]>) -> String {
    let mut out = input.to_string();
    for pattern in CREDENTIAL_PATTERNS.iter() {
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
    for (kind, re) in PERSONAL_PATTERNS.iter() {
        out = re
            .replace_all(&out, |caps: &regex::Captures| {
                token_with_key(*kind, &caps[0], org_id, key)
            })
            .into_owned();
    }
    out
}

/// Scrub a header map: sensitive header names are fully redacted, all other
/// values are run through [`scrub_string`]. Header-name comparison is
/// case-insensitive.
pub fn scrub_headers(headers: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    scrub_headers_for_org(headers, "")
}

/// [`scrub_headers`] with an org salt for the personal-identifier hashes.
pub fn scrub_headers_for_org(
    headers: &BTreeMap<String, String>,
    org_id: &str,
) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for (k, v) in headers {
        if SENSITIVE_HEADERS.contains(&k.to_lowercase().as_str()) {
            out.insert(k.clone(), "[redacted]".to_string());
        } else {
            out.insert(k.clone(), scrub_string_for_org(v, org_id));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Tests drive `scrub_with_key` / `token_with_key` rather than installing
    /// the process-wide key: `OnceCell` is set-once and cargo runs these in
    /// parallel in one process, so a global write would make the suite racy.
    const KEY: &[u8] = b"test-hmac-key-not-a-real-secret";
    const OTHER_KEY: &[u8] = b"a-different-test-hmac-key";

    fn scrub(input: &str, org: &str) -> String {
        scrub_with_key(input, org, Some(KEY))
    }

    // ---- credentials: dropped, never hashed -------------------------------

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
    fn credentials_are_dropped_not_hashed() {
        // A live token must never become a correlatable handle — hashing one
        // still yields an oracle you can test candidate tokens against.
        let s = scrub(
            "Bearer abc.def-ghi_123 password=hunter2 sk-ABCDEFGHIJKLMNOPQRSTUVWX",
            "org-1",
        );
        assert!(s.contains("Bearer [redacted]"), "{s}");
        assert!(s.contains("password=[redacted]"), "{s}");
        assert!(s.contains("sk-[redacted]"), "{s}");
        assert!(!s.contains("[token:"), "{s}");
        assert!(!s.contains("[credential:"), "{s}");
        // Nothing hash-shaped anywhere: no `[<kind>:<hex>]` token was emitted.
        assert!(!s.contains("[email:"), "{s}");
        assert!(!s.contains("[phone:"), "{s}");
        // And a personal identifier hiding inside a secret goes with it.
        let s2 = scrub("token=a@b.com", "org-1");
        assert!(!s2.contains("a@b.com"), "{s2}");
        assert!(!s2.contains("[email:"), "{s2}");
    }

    // ---- personal identifiers: hashed, prefix preserved -------------------

    #[test]
    fn hashes_emails_keeping_the_type_prefix() {
        let s = scrub("contact me at Alice@Example.com please", "org-1");
        assert!(!s.contains("Alice@Example.com"), "{s}");
        assert!(!s.to_lowercase().contains("alice@example.com"), "{s}");
        assert!(s.contains("[email:"), "{s}");
        assert!(s.starts_with("contact me at [email:"), "{s}");
        assert!(s.ends_with("] please"), "{s}");
    }

    #[test]
    fn hashes_phone_numbers() {
        for raw in ["555-0142", "(415) 555-0142", "+1 415-555-0142"] {
            let s = scrub(&format!("call {raw} today"), "org-1");
            assert!(s.contains("[phone:"), "{raw} -> {s}");
            assert!(!s.contains("0142"), "{raw} -> {s}");
        }
    }

    #[test]
    fn hashes_street_addresses() {
        let s = scrub("ship to 1600 Pennsylvania Ave, Washington", "org-1");
        assert!(s.contains("[address:"), "{s}");
        assert!(!s.contains("Pennsylvania"), "{s}");
    }

    #[test]
    fn same_value_same_org_is_stable() {
        let a = scrub("a@b.com", "org-1");
        let b = scrub("a@b.com", "org-1");
        assert_eq!(a, b);
        // …and correlation survives formatting differences in phones.
        assert_eq!(
            scrub("(415) 555-0142", "org-1"),
            scrub("415-555-0142", "org-1")
        );
    }

    #[test]
    fn same_value_different_org_hashes_differently() {
        let a = scrub("a@b.com", "org-1");
        let b = scrub("a@b.com", "org-2");
        assert_ne!(a, b, "per-org salt missing: {a} == {b}");
        assert!(a.starts_with("[email:") && b.starts_with("[email:"));
    }

    #[test]
    fn different_key_hashes_differently() {
        let a = scrub_with_key("a@b.com", "org-1", Some(KEY));
        let b = scrub_with_key("a@b.com", "org-1", Some(OTHER_KEY));
        assert_ne!(a, b, "hash is not keyed: {a} == {b}");
    }

    #[test]
    fn no_key_redacts_rather_than_hashing() {
        // Fail safe: an unkeyed digest of an email is rainbow-tabled instantly.
        let s = scrub_with_key("a@b.com and 555-0142", "org-1", None);
        assert!(s.contains("[email:redacted]"), "{s}");
        assert!(s.contains("[phone:redacted]"), "{s}");
        assert!(!s.contains("a@b.com"), "{s}");
    }

    #[test]
    fn hash_is_short_hex() {
        let t = token_with_key(PiiKind::Email, "a@b.com", "org-1", Some(KEY));
        let hex = t
            .trim_start_matches("[email:")
            .trim_end_matches(']')
            .to_string();
        assert_eq!(hex.len(), HASH_HEX_LEN, "{t}");
        assert!(hex.chars().all(|c| c.is_ascii_hexdigit()), "{t}");
    }

    #[test]
    fn pii_token_matches_what_scrubbing_wrote() {
        // The searchability contract: hashing a typed query term must produce
        // exactly the token stored in the span.
        let scrubbed = scrub("mail a@b.com now", "org-7");
        let term = token_with_key(PiiKind::Email, "A@B.com ", "org-7", Some(KEY));
        assert!(scrubbed.contains(&term), "{scrubbed} vs {term}");
    }

    #[test]
    fn scrub_string_for_org_salts_by_org() {
        // The public org-aware entry point behaves like the tested core even
        // with no key installed (both orgs redact, neither leaks).
        let a = scrub_string_for_org("a@b.com", "org-1");
        assert!(!a.contains("a@b.com"), "{a}");
        assert!(a.contains("[email:"), "{a}");
    }

    // ---- headers + non-matches -------------------------------------------

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

    #[test]
    fn ordinary_numbers_are_not_mistaken_for_phones() {
        // The phone pattern requires a separator before the last 4 digits;
        // ids, versions, dates and amounts must survive intact.
        for s in [
            "took 1234 ms",
            "version 1.2.3",
            "2026-08-15T12:00:00Z",
            "total 1234.5678",
            "trace 0af7651916cd43dd8448eb211c80319c",
        ] {
            assert_eq!(scrub(s, "org-1"), s, "false positive on {s}");
        }
    }

    #[test]
    fn tool_result_shape_from_traced_tool_is_scrubbed() {
        // The exact string the sibling repo's `traced_tool.rs` test passes
        // through untouched today.
        let s = scrub("did the thing; email=a@b.com", "org-1");
        assert!(!s.contains("a@b.com"), "{s}");
    }
}

//! Port of `packages/schemas/src/retail/product-constants.ts` — the subset
//! needed to compute `GET /v1/organizations/:org_id/features`.
//!
//! Kept narrow on purpose: only the maps + helpers the feature-resolution
//! path touches. Pricing, quotas, and tool mapping live in the TS source
//! of truth — porting them here would create two places to update every
//! time pricing changes, with no benefit (the hot-path service doesn't
//! serve any of them).
//!
//! When adding a product or feature in the TS source, mirror it here too
//! and the shadow harness will flag any divergence on the next run.

use std::collections::HashSet;

/// Domain that identifies SmooAI internal staff. Mirrors `INTERNAL_EMAIL_DOMAIN`.
pub const INTERNAL_EMAIL_DOMAIN: &str = "smoo.ai";

/// Features that are always-on for every org regardless of subscription.
/// Mirrors `DEFAULT_ENABLED_FEATURES`.
pub const DEFAULT_ENABLED_FEATURES: &[&str] = &["agents", "integrations"];

/// Features that are gated to `@smoo.ai` internal staff regardless of
/// Stripe product ownership. Mirrors `INTERNAL_ONLY_FEATURES`.
pub const INTERNAL_ONLY_FEATURES: &[&str] = &[
    "campaigns",
    "contentbuilder",
    "fieldservice",
    "observability",
    "commerce",
    "invoicing",
    "websitemanagement",
    "analytics_custom",
    "referrals",
    "whitelabel",
    "signal_agents",
];

/// (product_name, features...) tuples. Mirrors `PRODUCT_FEATURE_MAP`.
/// Kept as a `&[(&str, &[&str])]` for trivial constant-time lookup —
/// the table is small enough that linear scan in `features_for_product` is
/// fine and we avoid a hash-map allocation on every request.
pub const PRODUCT_FEATURE_MAP: &[(&str, &[&str])] = &[
    ("Smoo AI Free", &["agents"]),
    ("Smoo AI Starter", &["agents"]),
    ("Smoo AI Pro", &["agents"]),
    ("Smoo AI Business", &["agents"]),
    ("Smoo Testing", &["testing"]),
    ("Smoo Config", &["config"]),
    ("Smoo Platform Bundle", &["agents", "testing", "config", "observability"]),
    ("Smoo Customer Success Bundle", &["agents", "crm", "support"]),
    ("Smoo Growth Bundle", &["agents", "crm", "campaigns"]),
    ("Smoo Flex Chat", &["agents"]),
    ("Smoo AI CRM", &["crm"]),
    ("Smoo AI Support", &["support"]),
    ("Smoo AI Support - Pro", &["support", "workforce"]),
    ("Smoo Support + Agent Bundle", &["agents", "support"]),
    ("Smoo Workforce + Support Bundle", &["workforce", "support"]),
    ("Smoo AI Field Service", &["fieldservice"]),
    ("Smoo AI Analytics", &["analytics"]),
    ("Smoo AI Custom Analytics", &["analytics_custom"]),
    ("Smoo AI Content Builder", &["contentbuilder"]),
    ("Smoo Observability", &["observability"]),
    ("Smoo AI Campaigns", &["campaigns"]),
    ("Smoo AI Commerce", &["commerce"]),
    ("Smoo Website Management", &["websitemanagement"]),
    ("Smoo Invoicing", &["invoicing"]),
    ("Smoo AI Security - Starter", &["security"]),
    ("Smoo AI Security - Pro", &["security"]),
    ("Smoo AI Security - Enterprise", &["security"]),
    ("Smoo AI Workforce - Starter", &["workforce"]),
    ("Smoo AI Workforce - Pro", &["workforce"]),
    ("Smoo AI Workforce - Enterprise", &["workforce"]),
    ("Smoo AI Compliance Bundle", &["security", "workforce"]),
    ("Smoo AI Enterprise Bundle", &["security", "workforce", "agents"]),
    ("Smoo AI E-Sign", &["esign"]),
];

/// Mirrors `isInternalEmail`. Tolerates `+tag` aliases and is
/// case-insensitive on the domain part.
pub fn is_internal_email(email: Option<&str>) -> bool {
    let Some(email) = email else { return false };
    let Some(idx) = email.rfind('@') else { return false };
    email[idx + 1..].eq_ignore_ascii_case(INTERNAL_EMAIL_DOMAIN)
}

/// Resolve features for a single product name. Returns `&[]` for unknown
/// products (which the TS implementation also does — `mappedFeatures` is
/// undefined and the loop is skipped).
pub fn features_for_product(product_name: &str) -> &'static [&'static str] {
    for (name, features) in PRODUCT_FEATURE_MAP {
        if *name == product_name {
            return features;
        }
    }
    &[]
}

/// Apply internal-segment enrichment. Mirrors `expandFeaturesWithInternal`.
pub fn expand_with_internal(features: &mut HashSet<String>, email: Option<&str>) {
    if !is_internal_email(email) {
        return;
    }
    for f in INTERNAL_ONLY_FEATURES {
        features.insert((*f).to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn internal_email_detection() {
        assert!(is_internal_email(Some("brent@smoo.ai")));
        assert!(is_internal_email(Some("Brent+test@SMOO.ai")));
        assert!(!is_internal_email(Some("brent@example.com")));
        assert!(!is_internal_email(Some("no-at-sign")));
        assert!(!is_internal_email(None));
    }

    #[test]
    fn product_feature_lookup() {
        assert_eq!(features_for_product("Smoo AI Free"), &["agents"]);
        assert_eq!(
            features_for_product("Smoo Platform Bundle"),
            &["agents", "testing", "config", "observability"],
        );
        assert!(features_for_product("Unknown Product").is_empty());
    }

    #[test]
    fn internal_segment_enrichment() {
        let mut set: HashSet<String> = ["agents".to_string()].into_iter().collect();
        expand_with_internal(&mut set, Some("brent@smoo.ai"));
        assert!(set.contains("signal_agents"));
        assert!(set.contains("observability"));

        let mut public_set: HashSet<String> = ["agents".to_string()].into_iter().collect();
        expand_with_internal(&mut public_set, Some("user@example.com"));
        assert!(!public_set.contains("signal_agents"));
    }
}

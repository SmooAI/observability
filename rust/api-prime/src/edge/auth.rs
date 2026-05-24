//! Edge auth — JWT (user) + M2M token verification.
//!
//! Re-uses [`crate::auth::jwt::JwksCache`] for both Supabase user JWTs
//! and SST Auth M2M tokens (same JWKS shape; the M2M issuer is the SST
//! Auth component, see `packages/auth/src/server/m2m.ts` in the smooai
//! monorepo). We distinguish them by claims: M2M tokens carry
//! `role = service_role` or have `m2m` in `aud`; user tokens carry
//! `role = authenticated`.
//!
//! Invalid tokens log a structured warning + return 401. The
//! controller's audit pipeline consumes these via tracing.

use axum::http::HeaderMap;
use serde::{Deserialize, Serialize};
use tracing::warn;

use crate::auth::jwt::{extract_bearer, Claims, JwksCache};
use crate::edge::types::{AuthRequirement, RouteEntry};
use crate::error::AppError;

/// What the rest of the pipeline knows about the caller.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AuthKind {
    User { user_id: String },
    M2m { client_id: String },
    Public,
}

impl AuthKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            AuthKind::User { .. } => "user",
            AuthKind::M2m { .. } => "m2m",
            AuthKind::Public => "public",
        }
    }
}

/// Output of the edge auth stage.
#[derive(Debug, Clone)]
pub struct EdgeAuthContext {
    pub sub: String,
    pub kind: AuthKind,
    pub raw_jwt: Option<String>,
}

pub async fn verify(
    headers: &HeaderMap,
    peer_addr: &str,
    route: &RouteEntry,
    jwks: &JwksCache,
) -> Result<EdgeAuthContext, AppError> {
    match route.auth {
        AuthRequirement::Public => Ok(EdgeAuthContext {
            sub: format!("anon:{peer_addr}"),
            kind: AuthKind::Public,
            raw_jwt: None,
        }),
        AuthRequirement::User | AuthRequirement::M2m => {
            let auth_header = headers
                .get(axum::http::header::AUTHORIZATION)
                .and_then(|v| v.to_str().ok());
            let token = extract_bearer(auth_header).map_err(audit_unauthorized("missing bearer"))?;
            let claims = jwks
                .verify(token)
                .await
                .map_err(audit_unauthorized("jwt verify failed"))?;
            let kind = classify(&claims);
            if !matches_requirement(&kind, route.auth) {
                warn!(route = %route.path, required = ?route.auth, got = kind.as_str(), "auth kind mismatch");
                return Err(AppError::Unauthorized("auth kind does not satisfy route requirement".to_string()));
            }
            Ok(EdgeAuthContext {
                sub: claims.sub.clone(),
                kind,
                raw_jwt: Some(token.to_string()),
            })
        }
    }
}

fn classify(claims: &Claims) -> AuthKind {
    let role = claims.role.as_deref().unwrap_or("");
    let aud_is_m2m = claims.aud.contains("m2m") || claims.aud == "service";
    if role == "service_role" || aud_is_m2m {
        AuthKind::M2m {
            client_id: claims.sub.clone(),
        }
    } else {
        AuthKind::User {
            user_id: claims.sub.clone(),
        }
    }
}

fn matches_requirement(kind: &AuthKind, required: AuthRequirement) -> bool {
    matches!(
        (kind, required),
        (AuthKind::User { .. }, AuthRequirement::User)
            | (AuthKind::M2m { .. }, AuthRequirement::M2m)
            | (_, AuthRequirement::Public)
    )
}

fn audit_unauthorized(reason: &'static str) -> impl Fn(AppError) -> AppError {
    move |e| {
        warn!(reason = reason, error = %e, "edge auth rejected request");
        e
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn claims(role: &str, aud: &str) -> Claims {
        Claims {
            sub: "u-1".into(),
            aud: aud.into(),
            exp: 0,
            email: None,
            role: Some(role.into()),
            iss: None,
        }
    }

    #[test]
    fn classify_user_vs_m2m() {
        let c = claims("authenticated", "authenticated");
        assert!(matches!(classify(&c), AuthKind::User { .. }));

        let c = claims("service_role", "service");
        assert!(matches!(classify(&c), AuthKind::M2m { .. }));

        let c = claims("", "m2m");
        assert!(matches!(classify(&c), AuthKind::M2m { .. }));
    }

    #[test]
    fn requirement_matching() {
        assert!(matches_requirement(&AuthKind::User { user_id: "x".into() }, AuthRequirement::User));
        assert!(!matches_requirement(&AuthKind::User { user_id: "x".into() }, AuthRequirement::M2m));
        assert!(matches_requirement(&AuthKind::M2m { client_id: "c".into() }, AuthRequirement::M2m));
        assert!(matches_requirement(&AuthKind::Public, AuthRequirement::Public));
        assert!(matches_requirement(&AuthKind::User { user_id: "x".into() }, AuthRequirement::Public));
    }
}

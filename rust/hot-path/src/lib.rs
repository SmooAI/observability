//! Library surface for `smooai-hot-path`.
//!
//! The main binary lives in `src/main.rs`. This `lib.rs` re-exports the
//! modules so integration tests under `tests/` can construct an
//! `AppState` and exercise the router without going through `main`.

pub mod auth;
pub mod cache;
pub mod db;
pub mod error;
pub mod handlers;
pub mod product_constants;
pub mod state;

pub mod test_support {
    //! Router builder for integration tests.
    use axum::{
        routing::{get, post},
        Router,
    };

    use crate::{handlers, state::AppState};

    pub fn build_router(state: AppState) -> Router {
        Router::new()
            .route("/health/liveness", get(handlers::health::liveness))
            .route("/health/readiness", get(handlers::health::readiness))
            .route("/v1/profile", get(handlers::profile::get_profile))
            .route("/v1/auth/sign-in", post(handlers::auth::sign_in))
            .route("/v1/organizations", get(handlers::organizations::list_organizations))
            .route(
                "/v1/organizations/:org_id/features",
                get(handlers::organization_features::get_organization_features),
            )
            .route(
                "/v1/organizations/:org_id/products",
                get(handlers::organization_products::list_organization_products),
            )
            .with_state(state)
    }
}

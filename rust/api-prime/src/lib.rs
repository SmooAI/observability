//! Library surface for `smooai-api-prime`.
//!
//! Renamed from `smooai-hot-path` per ADR-017. Binaries live under
//! `src/bin/` (`api-prime.rs` for the data plane, `api-prime-controller.rs`
//! for the control plane). This `lib.rs` re-exports the modules so
//! integration tests under `tests/` can construct an `AppState` and
//! exercise the router without going through `main`.

pub mod auth;
pub mod cache;
pub mod controller;
pub mod db;
pub mod error;
pub mod handlers;
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
            .route("/v1/auth/refresh", post(handlers::auth::refresh))
            .with_state(state)
    }
}

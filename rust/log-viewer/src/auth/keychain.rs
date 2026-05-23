//! OS keychain wrapper for M2M `client_id` / `client_secret` pairs, one per
//! org. macOS Keychain on darwin, DPAPI / Credential Manager on Windows,
//! Secret Service (gnome-keyring / KWallet) on Linux — all via the `keyring`
//! crate.
//!
//! Layout:
//!
//! ```text
//! service:  smooai-observability-viewer
//! account:  {org_uuid}::client_id    -> the public client identifier
//! account:  {org_uuid}::client_secret -> the secret (sk_…)
//! ```
//!
//! We store the `client_id` in the keychain too (rather than alongside the
//! display name in plaintext config) so that even read access to the on-disk
//! state file doesn't leak which client a given org maps to.

use super::AuthError;
use uuid::Uuid;

const SERVICE: &str = "smooai-observability-viewer";

#[derive(Clone, Debug)]
pub struct Credentials {
    pub client_id: String,
    pub client_secret: String,
}

#[derive(Clone, Copy)]
pub struct Keychain;

impl Keychain {
    pub fn new() -> Self {
        Self
    }

    /// Store `(client_id, client_secret)` for `org`. Overwrites any existing
    /// values atomically (each entry write is its own keychain operation, but
    /// callers should treat the pair as a single unit).
    pub fn store(&self, org: Uuid, creds: &Credentials) -> Result<(), AuthError> {
        keyring::Entry::new(SERVICE, &account(org, "client_id"))?
            .set_password(&creds.client_id)?;
        keyring::Entry::new(SERVICE, &account(org, "client_secret"))?
            .set_password(&creds.client_secret)?;
        Ok(())
    }

    pub fn get(&self, org: Uuid) -> Result<Credentials, AuthError> {
        let client_id = keyring::Entry::new(SERVICE, &account(org, "client_id"))?
            .get_password()
            .map_err(|e| match e {
                keyring::Error::NoEntry => AuthError::MissingCredentials(org),
                other => other.into(),
            })?;
        let client_secret = keyring::Entry::new(SERVICE, &account(org, "client_secret"))?
            .get_password()
            .map_err(|e| match e {
                keyring::Error::NoEntry => AuthError::MissingCredentials(org),
                other => other.into(),
            })?;
        Ok(Credentials { client_id, client_secret })
    }

    pub fn remove(&self, org: Uuid) -> Result<(), AuthError> {
        // Best-effort — if either entry is missing, treat as already-gone.
        let _ = keyring::Entry::new(SERVICE, &account(org, "client_id"))?
            .delete_credential();
        let _ = keyring::Entry::new(SERVICE, &account(org, "client_secret"))?
            .delete_credential();
        Ok(())
    }
}

fn account(org: Uuid, field: &str) -> String {
    format!("{org}::{field}")
}

#[cfg(test)]
mod tests {
    //! These tests touch the real OS keychain and are off by default. Run with
    //! `cargo test -p smooai-observability-viewer --features keychain-tests`
    //! when iterating locally; CI skips them so we don't pollute hosted runner
    //! keychains.

    #[test]
    fn account_keys_are_disjoint() {
        let org = uuid::Uuid::new_v4();
        assert_ne!(super::account(org, "client_id"), super::account(org, "client_secret"));
        assert!(super::account(org, "client_id").starts_with(&org.to_string()));
    }
}

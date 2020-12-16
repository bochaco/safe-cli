// Copyright 2020 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under the MIT license <LICENSE-MIT
// http://opensource.org/licenses/MIT> or the Modified BSD license <LICENSE-BSD
// https://opensource.org/licenses/BSD-3-Clause>, at your option. This file may not be copied,
// modified, or distributed except according to those terms. Please review the Licences for the
// specific language governing permissions and limitations relating to use of the SAFE Network
// Software.

use crate::{Result, Safe, SecretKey};
use rand::distributions::Alphanumeric;
use rand::{thread_rng, Rng};
use std::env::var;

// Environment variable which can be set with the auth credentials
// to be used for all sn_api tests
const TEST_AUTH_CREDENTIALS: &str = "TEST_AUTH_CREDENTIALS";

// Instantiate a Safe instance
pub async fn new_safe_instance() -> Result<Safe> {
    let mut safe = Safe::default();
    let credentials = match var(TEST_AUTH_CREDENTIALS) {
        Ok(val) => val,
        Err(_) => "fake-credentials-string".to_string(),
    };

    safe.connect(Some(&credentials)).await?;
    Ok(safe)
}

// Create a random NRS name
pub fn random_nrs_name() -> String {
    thread_rng().sample_iter(&Alphanumeric).take(15).collect()
}

// Get hex string of a SecretKey
pub fn sk_to_hex_str(sk: SecretKey) -> String {
    match sk {
        SecretKey::Ed25519(sk) => sk.to_bytes().iter().map(|b| format!("{:02x}", b)).collect(),
        SecretKey::Bls(sk) => sk.inner().reveal(),
        SecretKey::BlsShare(sk) => sk.inner().reveal(),
    }
}

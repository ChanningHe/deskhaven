//! Multi-step client flows (logout then 401, 2FA two-step, OIDC polling,
//! nonce dedupe, expiry) do not fit one request per fixture: they are Rust
//! tests on [`TestApp`], decoding every reply with the client mirror types.
//! This file is the template (see the crate README).
#![allow(clippy::unwrap_used)]

use deskhaven_contract::app::TestApp;
use deskhaven_contract::fixture::Request;
use deskhaven_contract::mirror::{ErrorBody, LoginOptions};

#[tokio::test]
async fn login_options_still_answers_after_a_rejected_method() {
    // Arrange: one app (one database, one AppState) for the whole flow.
    let app = TestApp::start().await.unwrap();

    // Act: step 1, a method the route does not serve.
    let rejected = app
        .send(&Request::new("POST", "/api/login-options"))
        .await
        .unwrap();
    // Step 2, the request the client actually makes.
    let options = app
        .send(&Request::new("GET", "/api/login-options"))
        .await
        .unwrap();

    // Assert: each reply as the client reads it.
    assert_eq!(rejected.status, 405);
    assert_eq!(
        rejected.json::<ErrorBody>().unwrap().error,
        "method not allowed"
    );
    assert_eq!(options.status, 200);
    assert!(options.json::<LoginOptions>().unwrap().is_empty());
}

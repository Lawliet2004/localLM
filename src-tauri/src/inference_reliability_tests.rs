use super::*;
use crate::subscription_auth::{save_session, SubscriptionSession, PROVIDER_CHATGPT};
use serde_json::json;

#[tokio::test]
async fn long_turn_uses_rotated_credentials_and_rejects_account_switch() {
    let root = tempfile::tempdir().unwrap();
    let vault = std::sync::Arc::new(crate::vault::Vault::new(root.path().into()));
    let mut session = SubscriptionSession {
        provider: PROVIDER_CHATGPT.into(),
        access_token: "initial-fixture-secret".into(),
        refresh_token: None,
        expires_at: Some(crate::store::now() / 1000 + 3600),
        account_id: Some("fixture-account".into()),
        plan_type: None,
        source: "fixture".into(),
        updated_at: crate::store::now(),
    };
    save_session(&vault, &session).unwrap();
    let (base, requests, server) =
        crate::test_support::scripted_sse(vec!["data: [DONE]\n\n".into(); 2]).await;
    let model = crate::providers::RemoteModel {
        controls: None,
        id: "fixture-model".into(),
        context_length: Some(8192),
        max_output_tokens: Some(512),
        supports_images: false,
        tool_support: crate::providers::ToolSupport::Supported,
    };
    let backend = Backend::subscription(
        "Fixture",
        &base,
        session.access_token.clone(),
        session.account_id.clone(),
        &model,
    )
    .unwrap()
    .with_subscription_auth(vault.clone(), PROVIDER_CHATGPT);
    backend
        .stream(&json!({"messages":[]}))
        .await
        .unwrap()
        .bytes()
        .await
        .unwrap();
    session.access_token = "rotated-fixture-secret".into();
    save_session(&vault, &session).unwrap();
    backend
        .stream(&json!({"messages":[]}))
        .await
        .unwrap()
        .bytes()
        .await
        .unwrap();
    server.await.unwrap();
    let sent = requests.lock().unwrap();
    assert_eq!(sent.len(), 2);
    assert!(sent[0]
        .headers
        .to_ascii_lowercase()
        .contains("authorization: bearer initial-fixture-secret"));
    assert!(sent[1]
        .headers
        .to_ascii_lowercase()
        .contains("authorization: bearer rotated-fixture-secret"));
    let error =
        backend.stream_error(&json!({"message":"initial-fixture-secret rotated-fixture-secret"}));
    assert!(!error.contains("initial-fixture-secret") && !error.contains("rotated-fixture-secret"));
    drop(sent);
    session.account_id = Some("another-account".into());
    save_session(&vault, &session).unwrap();
    let error = backend.stream(&json!({"messages":[]})).await.unwrap_err();
    assert!(error.contains("account changed"));
    assert_eq!(
        requests.lock().unwrap().len(),
        2,
        "an account switch must not dispatch a third request"
    );
}

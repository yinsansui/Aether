use crate::handlers::admin::request::AdminAppState;
use crate::handlers::admin::shared::{
    attach_admin_audit_response, mark_sensitive_admin_response_no_store,
};
use crate::handlers::shared::{decrypt_or_migrate_auth_api_key_secret, seal_auth_api_key_secret};
use crate::GatewayError;
use aether_oauth::identity::{providers::CustomOidcIdentityOAuthProvider, IdentityOAuthProvider};
use axum::{
    body::Body,
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::{json, Value};

// Serialize provisioning within this process without adding a database constraint.
static PROVISION_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[derive(serde::Deserialize)]
struct EnsureRequest {
    coder_user_id: uuid::Uuid,
    claims: serde_json::Map<String, Value>,
}

pub(super) async fn ensure_api_key(
    state: &AdminAppState<'_>,
    body: Option<&axum::body::Bytes>,
) -> Result<Response<Body>, GatewayError> {
    let payload = body.and_then(|body| serde_json::from_slice::<EnsureRequest>(body).ok());
    let Some(payload) = payload.filter(|payload| !payload.coder_user_id.is_nil()) else {
        return Ok((
            StatusCode::BAD_REQUEST,
            Json(json!({"detail":"Invalid provisioning request"})),
        )
            .into_response());
    };
    if !state.has_auth_user_write_capability()
        || !state.has_auth_wallet_write_capability()
        || !state.has_auth_api_key_writer()
    {
        return Ok(super::build_admin_users_read_only_response(
            "Provisioning unavailable",
        ));
    }
    let config = match crate::oauth::get_enabled_identity_oauth_provider_config(
        state.app(),
        "custom_oidc_oa",
    )
    .await
    {
        Ok(Some(config)) => config,
        _ => {
            return Ok((
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({"detail":"OAuth provider unavailable"})),
            )
                .into_response())
        }
    };
    let provider = CustomOidcIdentityOAuthProvider;
    let claims = CustomOidcIdentityOAuthProvider::identity_from_userinfo(
        &config,
        Value::Object(payload.claims),
    )
    .and_then(|identity| provider.map_identity(&config, identity));
    let Ok(claims) = claims else {
        return Ok((
            StatusCode::BAD_REQUEST,
            Json(json!({"detail":"Missing OAuth identity"})),
        )
            .into_response());
    };
    let _guard = PROVISION_LOCK.lock().await;
    let user = match crate::oauth::resolve_identity_oauth_login_user(state.app(), &claims).await {
        Ok(user) if user.is_active && !user.is_deleted => user,
        Ok(_) => {
            return Ok((
                StatusCode::FORBIDDEN,
                Json(json!({"detail":"User unavailable"})),
            )
                .into_response())
        }
        Err(error) => {
            return Ok((StatusCode::CONFLICT, Json(json!({"detail":error.code()}))).into_response())
        }
    };
    let mut records = state
        .list_auth_api_key_export_records_by_user_ids(std::slice::from_ref(&user.id))
        .await?;
    records.retain(|record| record.name.as_deref() == Some("abc-coder") && record.is_active);
    records.sort_by(|a, b| {
        (a.created_at_unix_secs, &a.api_key_id).cmp(&(b.created_at_unix_secs, &b.api_key_id))
    });
    let (id, key, created) = if let Some(record) = records.first() {
        if record
            .expires_at_unix_secs
            .is_some_and(|expiry| expiry <= chrono::Utc::now().timestamp() as u64)
        {
            return Ok((
                StatusCode::CONFLICT,
                Json(json!({"detail":"abc-coder key is expired"})),
            )
                .into_response());
        }
        let snapshots = state
            .list_auth_api_key_snapshots_by_ids(std::slice::from_ref(&record.api_key_id))
            .await?;
        if snapshots.iter().any(|snapshot| snapshot.api_key_is_locked) {
            return Ok((
                StatusCode::CONFLICT,
                Json(json!({"detail":"abc-coder key is locked"})),
            )
                .into_response());
        }
        let key = match decrypt_or_migrate_auth_api_key_secret(state.app(), record).await {
            Ok(key) => key,
            Err(_) => {
                return Ok((
                    StatusCode::CONFLICT,
                    Json(json!({"detail":"Existing key cannot be recovered"})),
                )
                    .into_response())
            }
        };
        (record.api_key_id.clone(), key, false)
    } else {
        let key = super::generate_admin_user_api_key_plaintext();
        let id = uuid::Uuid::new_v4().to_string();
        let hash = super::hash_admin_user_api_key(&key);
        let encrypted =
            match seal_auth_api_key_secret(state.app(), &user.id, &id, &hash, false, &key) {
                Ok(value) => value,
                Err(_) => {
                    return Ok((
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(json!({"detail":"Key encryption failed"})),
                    )
                        .into_response())
                }
            };
        let created = state
            .create_user_api_key(aether_data::repository::auth::CreateUserApiKeyRecord {
                user_id: user.id.clone(),
                api_key_id: id.clone(),
                key_hash: hash,
                key_encrypted: Some(encrypted),
                name: Some("abc-coder".to_string()),
                allowed_providers: None,
                allowed_api_formats: None,
                allowed_models: None,
                ip_rules: None,
                rate_limit: 0,
                concurrent_limit: None,
                force_capabilities: None,
                feature_settings: None,
                is_active: true,
                expires_at_unix_secs: None,
                auto_delete_on_expiry: false,
                total_requests: 0,
                total_tokens: 0,
                total_cost_usd: 0.0,
            })
            .await?;
        if created.is_none() {
            return Ok(super::build_admin_users_data_unavailable_response());
        }
        (id, key, true)
    };
    Ok(mark_sensitive_admin_response_no_store(attach_admin_audit_response(
        Json(json!({"aether_user_id":user.id,"api_key_id":id,"key_name":"abc-coder","key":key,"created_key":created})).into_response(),
        "admin_abc_coder_api_key_ensured", "ensure_abc_coder_api_key", "user_api_key", &id,
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use aether_data::repository::{
        auth::InMemoryAuthApiKeySnapshotRepository,
        oauth_providers::{InMemoryOAuthProviderRepository, StoredOAuthProviderConfig},
        users::{InMemoryUserReadRepository, UserReadRepository},
        wallet::InMemoryWalletRepository,
    };
    use std::sync::Arc;
    use tower::ServiceExt;

    #[tokio::test]
    async fn provisioning_reuses_interactive_oauth_identity_and_api_key() {
        let provider = StoredOAuthProviderConfig::new(
            "custom_oidc_oa".into(),
            "WeCom".into(),
            "client".into(),
            "https://gateway.example.test/callback".into(),
            "https://gateway.example.test/auth/callback".into(),
        )
        .unwrap()
        .with_config_fields(
            None,
            Some("https://idp.example.test/authorize".into()),
            Some("https://idp.example.test/token".into()),
            Some("https://idp.example.test/userinfo".into()),
            None,
            Some(json!({"sub":"sub"})),
            Some(json!({"allowed_domains":["idp.example.test"]})),
            None,
            true,
        );
        let users = Arc::new(InMemoryUserReadRepository::default());
        let data = crate::data::GatewayDataState::with_user_and_wallet_for_tests(
            users.clone(),
            Arc::new(InMemoryWalletRepository::seed(vec![])),
        )
        .attach_oauth_provider_repository_for_tests(Arc::new(
            InMemoryOAuthProviderRepository::seed([provider]),
        ))
        .attach_auth_api_key_repository_for_tests(Arc::new(
            InMemoryAuthApiKeySnapshotRepository::seed([]),
        ))
        .with_encryption_key_for_tests(aether_crypto::DEVELOPMENT_ENCRYPTION_KEY)
        .with_system_config_values_for_tests(vec![
            ("module.oauth.enabled".into(), json!(true)),
            ("enable_registration".into(), json!(true)),
        ]);
        let app = crate::AppState::new()
            .unwrap()
            .with_data_state_for_tests(data);
        let state = AdminAppState::new(&app);
        let body = axum::body::Bytes::from(serde_json::to_vec(&json!({
            "coder_user_id":uuid::Uuid::new_v4(), "claims":{"sub":"broker-123","username":"alice"}
        })).unwrap());
        let router = crate::build_router_with_state(app.clone());
        let request = || {
            axum::http::Request::builder()
                .method("POST")
                .uri("/api/admin/integrations/abc-coder/api-key/ensure")
                .extension(axum::extract::ConnectInfo(std::net::SocketAddr::from((
                    [127, 0, 0, 1],
                    4242,
                ))))
                .header("content-type", "application/json")
        };
        let denied = router
            .clone()
            .oneshot(request().body(Body::from(body.clone())).unwrap())
            .await
            .unwrap();
        assert_eq!(denied.status(), StatusCode::UNAUTHORIZED);
        let first = router
            .oneshot(
                request()
                    .header(crate::constants::GATEWAY_HEADER, "rust-phase3b")
                    .header(
                        crate::constants::TRUSTED_ADMIN_USER_ID_HEADER,
                        "admin-user-123",
                    )
                    .header(crate::constants::TRUSTED_ADMIN_USER_ROLE_HEADER, "admin")
                    .header(
                        crate::constants::TRUSTED_ADMIN_SESSION_ID_HEADER,
                        "session-123",
                    )
                    .body(Body::from(body.clone()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(first.status(), StatusCode::OK);
        assert!(first
            .headers()
            .get(http::header::CACHE_CONTROL)
            .unwrap()
            .to_str()
            .unwrap()
            .contains("no-store"));
        let first: Value = serde_json::from_slice(
            &axum::body::to_bytes(first.into_body(), 64 << 10)
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(first["created_key"], true);
        let config =
            crate::oauth::get_enabled_identity_oauth_provider_config(&app, "custom_oidc_oa")
                .await
                .unwrap()
                .unwrap();
        let identity = CustomOidcIdentityOAuthProvider::identity_from_userinfo(
            &config,
            json!({"sub":"broker-123","username":"alice"}),
        )
        .unwrap();
        let claims = CustomOidcIdentityOAuthProvider
            .map_identity(&config, identity)
            .unwrap();
        let interactive = crate::oauth::resolve_identity_oauth_login_user(&app, &claims)
            .await
            .unwrap();
        assert_eq!(first["aether_user_id"], interactive.id);
        let second = ensure_api_key(&state, Some(&body)).await.unwrap();
        assert_eq!(second.status(), StatusCode::OK);
        let second: Value = serde_json::from_slice(
            &axum::body::to_bytes(second.into_body(), 64 << 10)
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(second["created_key"], false);
        assert_eq!(second["api_key_id"], first["api_key_id"]);
        assert_eq!(second["key"], first["key"]);
        assert!(users
            .find_oauth_linked_user("custom_oidc_oa", "broker-123")
            .await
            .unwrap()
            .is_some());
    }
}

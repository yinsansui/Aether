use async_trait::async_trait;
use std::net::IpAddr;
use url::{Host, Url};

/// 私网部署的显式开关。置为真时，`redirect_uri` 与 `frontend_callback_url`
/// 额外允许私网或回环 IP 字面量走 http；默认关闭，默认策略仍是
/// 「https，或回环地址上的 http」。
const OAUTH_ALLOW_PRIVATE_HTTP_ENV: &str = "AETHER_OAUTH_ALLOW_PRIVATE_HTTP";

fn oauth_private_http_allowed() -> bool {
    std::env::var(OAUTH_ALLOW_PRIVATE_HTTP_ENV)
        .ok()
        .is_some_and(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
}

/// 判定 IP 字面量是否落在可显式放开明文 http 的私网范围：IPv4 为 RFC1918
/// 私网与回环，IPv6 为回环与 ULA。刻意不含 `0.0.0.0`/`::` 与链路本地地址
/// （含云元数据 `169.254.169.254`），避免这个开关顺带放行它们。
fn is_private_or_loopback_ip(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(address) => address.is_private() || address.is_loopback(),
        IpAddr::V6(address) => {
            if let Some(mapped) = address.to_ipv4_mapped() {
                return is_private_or_loopback_ip(IpAddr::V4(mapped));
            }
            address.is_loopback() || address.is_unique_local()
        }
    }
}

fn oauth_transport_allowed(host: Host<&str>, scheme: &str, allow_private_http: bool) -> bool {
    if scheme == "https" {
        return true;
    }
    if scheme != "http" {
        return false;
    }
    match host {
        Host::Domain(domain) => domain.eq_ignore_ascii_case("localhost"),
        Host::Ipv4(address) => {
            address.is_loopback()
                || (allow_private_http && is_private_or_loopback_ip(IpAddr::V4(address)))
        }
        Host::Ipv6(address) => {
            address.is_loopback()
                || (allow_private_http && is_private_or_loopback_ip(IpAddr::V6(address)))
        }
    }
}

pub fn validate_oauth_redirect_uri(value: &str) -> Result<(), String> {
    let parsed =
        Url::parse(value).map_err(|_| "redirect_uri must be an absolute URL".to_string())?;
    let Some(host) = parsed.host() else {
        return Err("redirect_uri must be an absolute URL".to_string());
    };
    if !oauth_transport_allowed(host, parsed.scheme(), oauth_private_http_allowed()) {
        return Err(
            "redirect_uri must use https, except for localhost or loopback IPs".to_string(),
        );
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err("redirect_uri must not contain URL credentials".to_string());
    }
    if parsed.fragment().is_some() {
        return Err("redirect_uri must not contain a fragment".to_string());
    }
    Ok(())
}

pub fn validate_oauth_frontend_callback_url(value: &str) -> Result<(), String> {
    let parsed = Url::parse(value)
        .map_err(|_| "frontend_callback_url must be an absolute URL".to_string())?;
    let Some(host) = parsed.host() else {
        return Err("frontend_callback_url must be an absolute URL".to_string());
    };
    if !oauth_transport_allowed(host, parsed.scheme(), oauth_private_http_allowed()) {
        return Err(
            "frontend_callback_url must use https, except for localhost or loopback IPs"
                .to_string(),
        );
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err("frontend_callback_url must not contain URL credentials".to_string());
    }
    if parsed.query().is_some() || parsed.fragment().is_some() {
        return Err("frontend_callback_url must not contain a query or fragment".to_string());
    }
    if !parsed
        .path()
        .trim_end_matches('/')
        .ends_with("/auth/callback")
    {
        return Err("frontend_callback_url path must end with /auth/callback".to_string());
    }
    Ok(())
}

pub fn validate_oauth_provider_endpoint_config(
    provider_type: &str,
    authorization_url_override: Option<&str>,
    token_url_override: Option<&str>,
    userinfo_url_override: Option<&str>,
    extra_config: Option<&serde_json::Value>,
) -> Result<(), String> {
    let provider_type = provider_type.trim().to_ascii_lowercase();
    let mut provider_chars = provider_type.chars();
    if !(3..=64).contains(&provider_type.len())
        || !provider_chars
            .next()
            .is_some_and(|character| character.is_ascii_lowercase())
        || !provider_chars.all(|character| {
            character.is_ascii_lowercase()
                || character.is_ascii_digit()
                || matches!(character, '_' | '-')
        })
    {
        return Err("provider_type contains invalid characters".to_string());
    }
    let is_custom = provider_type == "custom_oidc"
        || provider_type.starts_with("custom_oidc_")
        || provider_type.starts_with("custom_")
        || provider_type.starts_with("oidc_");
    let allowed_domains = if provider_type == "linuxdo" {
        vec![
            "linux.do".to_string(),
            "connect.linux.do".to_string(),
            "connect.linuxdo.org".to_string(),
        ]
    } else if is_custom {
        oauth_custom_allowed_domains(extra_config)?
    } else {
        return Err("unsupported identity OAuth provider_type".to_string());
    };

    for (field, value) in [
        ("authorization_url_override", authorization_url_override),
        ("token_url_override", token_url_override),
        ("userinfo_url_override", userinfo_url_override),
    ] {
        let value = value.map(str::trim).filter(|value| !value.is_empty());
        if is_custom && value.is_none() {
            return Err(format!("custom OIDC providers must configure {field}"));
        }
        if let Some(value) = value {
            validate_oauth_endpoint_url(field, value, &allowed_domains)?;
        }
    }
    Ok(())
}

fn oauth_custom_allowed_domains(
    extra_config: Option<&serde_json::Value>,
) -> Result<Vec<String>, String> {
    let values = extra_config
        .and_then(serde_json::Value::as_object)
        .and_then(|object| {
            object
                .get("allowed_domains")
                .or_else(|| object.get("oauth_allowed_domains"))
        })
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| {
            "custom OIDC providers must configure extra_config.allowed_domains".to_string()
        })?;
    let mut domains = Vec::with_capacity(values.len());
    for value in values {
        let domain = value
            .as_str()
            .map(str::trim)
            .map(|value| value.trim_end_matches('.'))
            .filter(|value| !value.is_empty())
            .ok_or_else(|| "OAuth allowed_domains must contain only host names".to_string())?;
        if domain.contains('/')
            || domain.contains('\\')
            || domain.contains('@')
            || domain.contains(':')
            || domain.contains(char::is_whitespace)
            || domain.parse::<IpAddr>().is_ok()
        {
            return Err(
                "OAuth allowed_domains must contain DNS host names, not IP literals".to_string(),
            );
        }
        domains.push(domain.to_ascii_lowercase());
    }
    if domains.is_empty() {
        return Err("custom OIDC providers must configure allowed domains".to_string());
    }
    Ok(domains)
}

fn validate_oauth_endpoint_url(
    field: &str,
    value: &str,
    allowed_domains: &[String],
) -> Result<(), String> {
    let parsed = Url::parse(value).map_err(|_| format!("{field} must be an absolute URL"))?;
    if parsed.scheme() != "https" || parsed.host_str().is_none() {
        return Err(format!("{field} must be an absolute https URL"));
    }
    if matches!(parsed.host(), Some(Host::Ipv4(_)) | Some(Host::Ipv6(_))) {
        return Err(format!(
            "{field} must use a DNS host name, not an IP literal"
        ));
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(format!("{field} must not contain URL credentials"));
    }
    if parsed.fragment().is_some() {
        return Err(format!("{field} must not contain a fragment"));
    }
    if field == "authorization_url_override" {
        for (name, _) in parsed.query_pairs() {
            if matches!(
                name.to_ascii_lowercase().as_str(),
                "response_type"
                    | "client_id"
                    | "redirect_uri"
                    | "state"
                    | "scope"
                    | "code_challenge"
                    | "code_challenge_method"
            ) {
                return Err(format!(
                    "{field} must not predefine OAuth authorization parameters"
                ));
            }
        }
    }
    if !allowed_domains.is_empty() {
        let host = parsed
            .host_str()
            .map(|value| value.trim_end_matches('.').to_ascii_lowercase())
            .unwrap_or_default();
        if !allowed_domains
            .iter()
            .any(|domain| host == *domain || host.ends_with(&format!(".{domain}")))
        {
            return Err(format!("{field} host is not in the provider allowlist"));
        }
    }
    Ok(())
}

#[derive(Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct StoredOAuthProviderConfig {
    pub provider_type: String,
    pub display_name: String,
    pub client_id: String,
    pub client_secret_encrypted: Option<String>,
    pub authorization_url_override: Option<String>,
    pub token_url_override: Option<String>,
    pub userinfo_url_override: Option<String>,
    pub scopes: Option<Vec<String>>,
    pub redirect_uri: String,
    pub frontend_callback_url: String,
    pub attribute_mapping: Option<serde_json::Value>,
    pub extra_config: Option<serde_json::Value>,
    pub icon_url: Option<String>,
    pub is_enabled: bool,
    pub created_at_unix_ms: Option<u64>,
    pub updated_at_unix_secs: Option<u64>,
}

impl std::fmt::Debug for StoredOAuthProviderConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("StoredOAuthProviderConfig")
            .field("provider_type", &self.provider_type)
            .field("display_name", &self.display_name)
            .field("client_id", &self.client_id)
            .field(
                "client_secret_encrypted",
                &self.client_secret_encrypted.as_ref().map(|_| "[REDACTED]"),
            )
            .field(
                "authorization_url_override",
                &self
                    .authorization_url_override
                    .as_ref()
                    .map(|_| "[REDACTED]"),
            )
            .field(
                "token_url_override",
                &self.token_url_override.as_ref().map(|_| "[REDACTED]"),
            )
            .field(
                "userinfo_url_override",
                &self.userinfo_url_override.as_ref().map(|_| "[REDACTED]"),
            )
            .field("scopes", &self.scopes)
            .field("redirect_uri", &self.redirect_uri)
            .field("frontend_callback_url", &self.frontend_callback_url)
            .field(
                "attribute_mapping",
                &self.attribute_mapping.as_ref().map(|_| "[REDACTED]"),
            )
            .field(
                "extra_config",
                &self.extra_config.as_ref().map(|_| "[REDACTED]"),
            )
            .field("icon_url", &self.icon_url)
            .field("is_enabled", &self.is_enabled)
            .field("created_at_unix_ms", &self.created_at_unix_ms)
            .field("updated_at_unix_secs", &self.updated_at_unix_secs)
            .finish()
    }
}

impl StoredOAuthProviderConfig {
    pub fn new(
        provider_type: String,
        display_name: String,
        client_id: String,
        redirect_uri: String,
        frontend_callback_url: String,
    ) -> Result<Self, crate::DataLayerError> {
        if provider_type.trim().is_empty() {
            return Err(crate::DataLayerError::UnexpectedValue(
                "oauth_providers.provider_type is empty".to_string(),
            ));
        }
        if display_name.trim().is_empty() {
            return Err(crate::DataLayerError::UnexpectedValue(
                "oauth_providers.display_name is empty".to_string(),
            ));
        }
        if client_id.trim().is_empty() {
            return Err(crate::DataLayerError::UnexpectedValue(
                "oauth_providers.client_id is empty".to_string(),
            ));
        }
        if redirect_uri.trim().is_empty() {
            return Err(crate::DataLayerError::UnexpectedValue(
                "oauth_providers.redirect_uri is empty".to_string(),
            ));
        }
        if frontend_callback_url.trim().is_empty() {
            return Err(crate::DataLayerError::UnexpectedValue(
                "oauth_providers.frontend_callback_url is empty".to_string(),
            ));
        }

        Ok(Self {
            provider_type,
            display_name,
            client_id,
            client_secret_encrypted: None,
            authorization_url_override: None,
            token_url_override: None,
            userinfo_url_override: None,
            scopes: None,
            redirect_uri,
            frontend_callback_url,
            attribute_mapping: None,
            extra_config: None,
            icon_url: None,
            is_enabled: false,
            created_at_unix_ms: None,
            updated_at_unix_secs: None,
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn with_config_fields(
        mut self,
        client_secret_encrypted: Option<String>,
        authorization_url_override: Option<String>,
        token_url_override: Option<String>,
        userinfo_url_override: Option<String>,
        scopes: Option<Vec<String>>,
        attribute_mapping: Option<serde_json::Value>,
        extra_config: Option<serde_json::Value>,
        icon_url: Option<String>,
        is_enabled: bool,
    ) -> Self {
        self.client_secret_encrypted = client_secret_encrypted;
        self.authorization_url_override = authorization_url_override;
        self.token_url_override = token_url_override;
        self.userinfo_url_override = userinfo_url_override;
        self.scopes = scopes;
        self.attribute_mapping = attribute_mapping;
        self.extra_config = extra_config;
        self.icon_url = icon_url;
        self.is_enabled = is_enabled;
        self
    }

    pub fn with_timestamps(
        mut self,
        created_at_unix_ms: Option<u64>,
        updated_at_unix_secs: Option<u64>,
    ) -> Self {
        self.created_at_unix_ms = created_at_unix_ms;
        self.updated_at_unix_secs = updated_at_unix_secs;
        self
    }
}

#[derive(Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
pub enum EncryptedSecretUpdate {
    #[default]
    Preserve,
    Clear,
    Set(String),
}

impl std::fmt::Debug for EncryptedSecretUpdate {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Preserve => formatter.write_str("Preserve"),
            Self::Clear => formatter.write_str("Clear"),
            Self::Set(_) => formatter.write_str("Set([REDACTED])"),
        }
    }
}

impl EncryptedSecretUpdate {
    pub fn mode_name(&self) -> &'static str {
        match self {
            Self::Preserve => "preserve",
            Self::Clear => "clear",
            Self::Set(_) => "set",
        }
    }

    pub fn value(&self) -> Option<&str> {
        match self {
            Self::Set(value) => Some(value.as_str()),
            Self::Preserve | Self::Clear => None,
        }
    }
}

#[derive(Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct UpsertOAuthProviderConfigRecord {
    pub provider_type: String,
    pub display_name: String,
    pub client_id: String,
    pub client_secret_encrypted: EncryptedSecretUpdate,
    pub authorization_url_override: Option<String>,
    pub token_url_override: Option<String>,
    pub userinfo_url_override: Option<String>,
    pub scopes: Option<Vec<String>>,
    pub redirect_uri: String,
    pub frontend_callback_url: String,
    pub attribute_mapping: Option<serde_json::Value>,
    pub extra_config: Option<serde_json::Value>,
    pub icon_url: Option<String>,
    pub is_enabled: bool,
}

impl std::fmt::Debug for UpsertOAuthProviderConfigRecord {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("UpsertOAuthProviderConfigRecord")
            .field("provider_type", &self.provider_type)
            .field("display_name", &self.display_name)
            .field("client_id", &self.client_id)
            .field("client_secret_encrypted", &self.client_secret_encrypted)
            .field(
                "authorization_url_override",
                &self
                    .authorization_url_override
                    .as_ref()
                    .map(|_| "[REDACTED]"),
            )
            .field(
                "token_url_override",
                &self.token_url_override.as_ref().map(|_| "[REDACTED]"),
            )
            .field(
                "userinfo_url_override",
                &self.userinfo_url_override.as_ref().map(|_| "[REDACTED]"),
            )
            .field("scopes", &self.scopes)
            .field("redirect_uri", &"[REDACTED]")
            .field("frontend_callback_url", &"[REDACTED]")
            .field(
                "attribute_mapping",
                &self.attribute_mapping.as_ref().map(|_| "[REDACTED]"),
            )
            .field(
                "extra_config",
                &self.extra_config.as_ref().map(|_| "[REDACTED]"),
            )
            .field("icon_url", &self.icon_url)
            .field("is_enabled", &self.is_enabled)
            .finish()
    }
}

// Keep the returned provider value inline: this outcome is part of the
// repository API and the successful value is consumed immediately by callers.
// Boxing would be an API/ownership change for no security benefit.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq)]
pub enum UpsertOAuthProviderConfigOutcome {
    Upserted(StoredOAuthProviderConfig),
    DisableRequiresConfirmation { affected_count: usize },
}

impl UpsertOAuthProviderConfigRecord {
    pub fn validate(&self) -> Result<(), crate::DataLayerError> {
        if self.provider_type.trim().is_empty() {
            return Err(crate::DataLayerError::InvalidInput(
                "provider_type is required".to_string(),
            ));
        }
        if self.display_name.trim().is_empty() {
            return Err(crate::DataLayerError::InvalidInput(
                "display_name is required".to_string(),
            ));
        }
        if self.client_id.trim().is_empty() {
            return Err(crate::DataLayerError::InvalidInput(
                "client_id is required".to_string(),
            ));
        }
        if self.redirect_uri.trim().is_empty() {
            return Err(crate::DataLayerError::InvalidInput(
                "redirect_uri is required".to_string(),
            ));
        }
        validate_oauth_redirect_uri(self.redirect_uri.trim())
            .map_err(crate::DataLayerError::InvalidInput)?;
        if self.frontend_callback_url.trim().is_empty() {
            return Err(crate::DataLayerError::InvalidInput(
                "frontend_callback_url is required".to_string(),
            ));
        }
        validate_oauth_frontend_callback_url(self.frontend_callback_url.trim())
            .map_err(crate::DataLayerError::InvalidInput)?;
        validate_oauth_provider_endpoint_config(
            &self.provider_type,
            self.authorization_url_override.as_deref(),
            self.token_url_override.as_deref(),
            self.userinfo_url_override.as_deref(),
            self.extra_config.as_ref(),
        )
        .map_err(crate::DataLayerError::InvalidInput)?;
        if let Some(scopes) = &self.scopes {
            for scope in scopes {
                if scope.trim().is_empty() {
                    return Err(crate::DataLayerError::InvalidInput(
                        "scopes must not contain empty values".to_string(),
                    ));
                }
            }
        }
        Ok(())
    }
}

// Validation tests are kept beside the validation implementation so changes
// to endpoint policy are reviewed together. The repository traits below are
// intentionally declared after this focused test module for API readability.
#[allow(clippy::items_after_test_module)]
#[cfg(test)]
mod tests {
    use super::{
        validate_oauth_frontend_callback_url, validate_oauth_provider_endpoint_config,
        validate_oauth_redirect_uri, EncryptedSecretUpdate, StoredOAuthProviderConfig,
        OAUTH_ALLOW_PRIVATE_HTTP_ENV,
    };

    #[test]
    fn oauth_provider_debug_output_redacts_encrypted_client_secrets() {
        let secret = "debug-secret-oauth-provider-ciphertext";
        let provider = StoredOAuthProviderConfig::new(
            "linuxdo".to_string(),
            "Linux.do".to_string(),
            "client-id".to_string(),
            "https://gateway.example/api/oauth/linuxdo/callback".to_string(),
            "https://frontend.example/auth/callback".to_string(),
        )
        .expect("provider should build")
        .with_config_fields(
            Some(secret.to_string()),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            true,
        );

        for rendered in [
            format!("{provider:?}"),
            format!("{:?}", EncryptedSecretUpdate::Set(secret.to_string())),
        ] {
            assert!(!rendered.contains(secret));
            assert!(rendered.contains("[REDACTED]"));
        }
    }

    #[test]
    fn oauth_redirect_uri_requires_absolute_http_url_without_credentials() {
        assert!(
            validate_oauth_redirect_uri("https://gateway.example/api/oauth/custom/callback")
                .is_ok()
        );
        assert!(
            validate_oauth_redirect_uri("http://localhost:8080/api/oauth/custom/callback").is_ok()
        );
        for value in [
            "http://gateway.example/api/oauth/custom/callback",
            "/api/oauth/custom/callback",
            "javascript:alert(1)",
            "https://user:password@gateway.example/api/oauth/custom/callback",
        ] {
            assert!(
                validate_oauth_redirect_uri(value).is_err(),
                "accepted {value}"
            );
        }
    }

    #[test]
    fn oauth_provider_endpoints_require_https_and_the_configured_domain() {
        let extra = serde_json::json!({"allowed_domains": ["idp.example"]});
        assert!(validate_oauth_provider_endpoint_config(
            "custom_oidc_work",
            Some("https://idp.example/oauth/authorize"),
            Some("https://idp.example/oauth/token"),
            Some("https://accounts.idp.example/oauth/userinfo"),
            Some(&extra),
        )
        .is_ok());
        assert!(validate_oauth_provider_endpoint_config(
            "custom_oidc_work",
            Some("https://idp.example/oauth/authorize"),
            Some("https://attacker.example/oauth/token"),
            Some("https://idp.example/oauth/userinfo"),
            Some(&extra),
        )
        .is_err());
        assert!(validate_oauth_provider_endpoint_config(
            "custom_oidc_work",
            Some("https://127.0.0.1/oauth/authorize"),
            Some("https://idp.example/oauth/token"),
            Some("https://idp.example/oauth/userinfo"),
            Some(&serde_json::json!({"allowed_domains": ["127.0.0.1"]})),
        )
        .is_err());
        assert!(validate_oauth_provider_endpoint_config(
            "custom_oidc_work",
            Some("https://idp.example/oauth/authorize?client_id=attacker"),
            Some("https://idp.example/oauth/token"),
            Some("https://idp.example/oauth/userinfo"),
            Some(&extra),
        )
        .is_err());
        assert!(validate_oauth_provider_endpoint_config(
            "custom_oidc_work",
            Some("https://idp.example/oauth/authorize"),
            Some("http://idp.example/oauth/token"),
            Some("https://idp.example/oauth/userinfo"),
            Some(&extra),
        )
        .is_err());
    }

    #[test]
    fn oauth_provider_endpoints_reject_ip_literals_and_predefined_authorization_parameters() {
        for host in ["127.0.0.1", "[::1]"] {
            let extra = serde_json::json!({"allowed_domains": [host]});
            assert!(validate_oauth_provider_endpoint_config(
                "custom_oidc_work",
                Some(&format!("https://{host}/oauth/authorize")),
                Some(&format!("https://{host}/oauth/token")),
                Some(&format!("https://{host}/oauth/userinfo")),
                Some(&extra),
            )
            .is_err());
        }

        let extra = serde_json::json!({"allowed_domains": ["idp.example"]});
        for name in [
            "response_type",
            "client_id",
            "redirect_uri",
            "state",
            "scope",
            "code_challenge",
            "code_challenge_method",
        ] {
            assert!(validate_oauth_provider_endpoint_config(
                "custom_oidc_work",
                Some(&format!(
                    "https://idp.example/oauth/authorize?{name}=attacker"
                )),
                Some("https://idp.example/oauth/token?tenant=workforce"),
                Some("https://idp.example/oauth/userinfo?schema=current"),
                Some(&extra),
            )
            .is_err());
        }

        assert!(validate_oauth_provider_endpoint_config(
            "custom_oidc_work",
            Some("https://idp.example/oauth/authorize?tenant=workforce"),
            Some("https://idp.example/oauth/token?tenant=workforce"),
            Some("https://idp.example/oauth/userinfo?schema=current"),
            Some(&extra),
        )
        .is_ok());
    }

    #[test]
    fn oauth_frontend_callback_rejects_token_exfiltration_targets() {
        for value in [
            "https://frontend.example/auth/callback",
            "http://localhost:5173/auth/callback",
            "http://127.0.0.1:5173/auth/callback",
            "http://[::1]:5173/auth/callback",
        ] {
            assert!(
                validate_oauth_frontend_callback_url(value).is_ok(),
                "rejected {value}"
            );
        }
        for value in [
            "http://attacker.example/auth/callback",
            "https://user:password@frontend.example/auth/callback",
            "https://frontend.example/auth/callback?next=https://attacker.example",
            "https://frontend.example/auth/callback#access_token=stolen",
            "https://frontend.example/not-the-callback",
        ] {
            assert!(
                validate_oauth_frontend_callback_url(value).is_err(),
                "accepted {value}"
            );
        }
    }

    struct PrivateHttpEnvGuard {
        previous: Option<String>,
    }

    impl Drop for PrivateHttpEnvGuard {
        fn drop(&mut self) {
            match self.previous.take() {
                Some(value) => std::env::set_var(OAUTH_ALLOW_PRIVATE_HTTP_ENV, value),
                None => std::env::remove_var(OAUTH_ALLOW_PRIVATE_HTTP_ENV),
            }
        }
    }

    fn private_http_env_guard(value: Option<&str>) -> PrivateHttpEnvGuard {
        let previous = std::env::var(OAUTH_ALLOW_PRIVATE_HTTP_ENV).ok();
        match value {
            Some(value) => std::env::set_var(OAUTH_ALLOW_PRIVATE_HTTP_ENV, value),
            None => std::env::remove_var(OAUTH_ALLOW_PRIVATE_HTTP_ENV),
        }
        PrivateHttpEnvGuard { previous }
    }

    #[test]
    fn oauth_callback_transport_allows_private_http_only_when_opted_in() {
        // 默认策略：只有 https 与回环地址上的 http 通过，私网与公网字面量都拒绝。
        // 显式清空开关，避免外部环境已导出该变量时让这一段失去意义。
        let disabled_guard = private_http_env_guard(None);
        for rejected in [
            "http://frontend.example/auth/callback",
            "http://172.19.10.176:8084/auth/callback",
            "http://10.0.0.8:8084/auth/callback",
            "http://8.8.8.8:8084/auth/callback",
        ] {
            assert!(
                validate_oauth_frontend_callback_url(rejected).is_err(),
                "accepted {rejected} without opt-in"
            );
        }

        // AETHER_OAUTH_ALLOW_PRIVATE_HTTP=1 放行 RFC1918/ULA 与回环字面量，
        // 但不放行公网地址、域名，也不放行 0.0.0.0、:: 与链路本地地址。
        let enabled_guard = private_http_env_guard(Some("1"));
        assert!(validate_oauth_redirect_uri(
            "http://172.19.10.176:8084/api/oauth/custom_oidc_oa/callback"
        )
        .is_ok());
        assert!(validate_oauth_redirect_uri("http://10.0.0.8:8084/callback").is_ok());
        assert!(validate_oauth_redirect_uri("http://[fd00::1]:8084/callback").is_ok());
        assert!(
            validate_oauth_frontend_callback_url("http://172.19.10.176:8084/auth/callback").is_ok()
        );
        for rejected in [
            "http://8.8.8.8:8084/callback",
            "http://oa.abcyun.cn/callback",
            "http://[::ffff:8.8.8.8]:8084/callback",
            "http://0.0.0.0:8084/callback",
            "http://[::]:8084/callback",
            "http://169.254.169.254:8084/callback",
            "http://user:password@172.19.10.176:8084/callback",
        ] {
            assert!(
                validate_oauth_redirect_uri(rejected).is_err(),
                "accepted {rejected} with the opt-in"
            );
        }
        assert!(validate_oauth_frontend_callback_url("http://8.8.8.8:8084/auth/callback").is_err());
        assert!(validate_oauth_frontend_callback_url("http://oa.abcyun.cn/auth/callback").is_err());
        assert!(validate_oauth_frontend_callback_url("http://0.0.0.0:8084/auth/callback").is_err());
        drop(enabled_guard);
        drop(disabled_guard);
    }
}

#[async_trait]
pub trait OAuthProviderReadRepository: Send + Sync {
    async fn list_oauth_provider_configs(
        &self,
    ) -> Result<Vec<StoredOAuthProviderConfig>, crate::DataLayerError>;

    async fn get_oauth_provider_config(
        &self,
        provider_type: &str,
    ) -> Result<Option<StoredOAuthProviderConfig>, crate::DataLayerError>;

    async fn count_locked_users_if_provider_disabled(
        &self,
        provider_type: &str,
        ldap_exclusive: bool,
    ) -> Result<usize, crate::DataLayerError>;
}

#[async_trait]
pub trait OAuthProviderWriteRepository: Send + Sync {
    async fn upsert_oauth_provider_config(
        &self,
        record: &UpsertOAuthProviderConfigRecord,
    ) -> Result<StoredOAuthProviderConfig, crate::DataLayerError> {
        match self
            .upsert_oauth_provider_config_guarded(record, false, false, 0)
            .await?
        {
            UpsertOAuthProviderConfigOutcome::Upserted(provider) => Ok(provider),
            UpsertOAuthProviderConfigOutcome::DisableRequiresConfirmation { affected_count } => {
                Err(crate::DataLayerError::InvalidInput(format!(
                    "disabling OAuth provider requires confirmation for {affected_count} affected users"
                )))
            }
        }
    }

    async fn upsert_oauth_provider_config_guarded(
        &self,
        record: &UpsertOAuthProviderConfigRecord,
        ldap_exclusive: bool,
        force_disable: bool,
        locked_users_snapshot: usize,
    ) -> Result<UpsertOAuthProviderConfigOutcome, crate::DataLayerError>;

    /// Replace only the stored client secret when the provider and exact previously observed
    /// ciphertext still match. Implementations must not modify `updated_at` or any non-secret
    /// provider field; this is used by lazy record-bound ciphertext migration.
    async fn compare_and_swap_oauth_provider_client_secret(
        &self,
        provider_type: &str,
        expected: &str,
        replacement: &str,
    ) -> Result<bool, crate::DataLayerError>;

    async fn delete_oauth_provider_config_if_unlinked(
        &self,
        provider_type: &str,
        has_links_snapshot: bool,
    ) -> Result<bool, crate::DataLayerError>;
}

pub trait OAuthProviderRepository:
    OAuthProviderReadRepository + OAuthProviderWriteRepository + Send + Sync
{
}

impl<T> OAuthProviderRepository for T where
    T: OAuthProviderReadRepository + OAuthProviderWriteRepository + Send + Sync
{
}

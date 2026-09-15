use serde_json::Value;

/// Nested under `config.codex` alongside fingerprint convergence.
pub const CODEX_COMPACT_SYNTHESIS_CONFIG_NAMESPACE: &str = "codex";
pub const CODEX_COMPACT_SYNTHESIS_ENABLED_CONFIG_KEY: &str = "compact_synthesis_enabled";

/// Provider-level gate for Codex remote compaction v2 synthesis.
///
/// Unlike fingerprint convergence this is **not** limited to `provider_type=codex`:
/// any provider that Codex clients hit may need synthesis when the upstream cannot
/// emit a native `{ type: "compaction", encrypted_content }` item.
///
/// Default: `false` (GPT official paths stay untouched).
pub fn codex_compact_synthesis_enabled(provider_config: Option<&Value>) -> bool {
    provider_config
        .and_then(Value::as_object)
        .and_then(|config| config.get(CODEX_COMPACT_SYNTHESIS_CONFIG_NAMESPACE))
        .and_then(Value::as_object)
        .and_then(|codex| codex.get(CODEX_COMPACT_SYNTHESIS_ENABLED_CONFIG_KEY))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

pub fn set_codex_compact_synthesis_enabled(
    config_map: &mut serde_json::Map<String, Value>,
    enabled: bool,
) {
    let codex = config_map
        .entry(CODEX_COMPACT_SYNTHESIS_CONFIG_NAMESPACE.to_string())
        .or_insert_with(|| Value::Object(serde_json::Map::new()));
    if let Some(codex) = codex.as_object_mut() {
        codex.insert(
            CODEX_COMPACT_SYNTHESIS_ENABLED_CONFIG_KEY.to_string(),
            Value::Bool(enabled),
        );
    }
}

pub fn remove_codex_compact_synthesis_config(config_map: &mut serde_json::Map<String, Value>) {
    let Some(codex) = config_map
        .get_mut(CODEX_COMPACT_SYNTHESIS_CONFIG_NAMESPACE)
        .and_then(Value::as_object_mut)
    else {
        return;
    };
    codex.remove(CODEX_COMPACT_SYNTHESIS_ENABLED_CONFIG_KEY);
    if codex.is_empty() {
        config_map.remove(CODEX_COMPACT_SYNTHESIS_CONFIG_NAMESPACE);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn defaults_to_disabled() {
        assert!(!codex_compact_synthesis_enabled(None));
        assert!(!codex_compact_synthesis_enabled(Some(&json!({}))));
        assert!(!codex_compact_synthesis_enabled(Some(
            &json!({"codex": {"fingerprint_convergence_enabled": true}})
        )));
        assert!(!codex_compact_synthesis_enabled(Some(
            &json!({"codex": {"compact_synthesis_enabled": false}})
        )));
    }

    #[test]
    fn reads_enabled_flag_for_any_provider_config() {
        assert!(codex_compact_synthesis_enabled(Some(
            &json!({"codex": {"compact_synthesis_enabled": true}})
        )));
    }

    #[test]
    fn set_and_remove_roundtrip() {
        let mut config = serde_json::Map::new();
        set_codex_compact_synthesis_enabled(&mut config, true);
        assert_eq!(
            config.get("codex").and_then(|v| v.get("compact_synthesis_enabled")),
            Some(&json!(true))
        );
        remove_codex_compact_synthesis_config(&mut config);
        assert!(config.is_empty());
    }
}

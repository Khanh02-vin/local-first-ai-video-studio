// The provider branch: which AI provider the analysis uses and how it is configured.
//
// The provider list lives in a small JSON store next to the app state; the active
// provider is flattened into the runtime environment the worker reads.
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};


use crate::paths::{
    runtime_env, data_dir, cache_dir, valid_local_llm_url, valid_provider_url, whisper_command,
    whisper_usable, write_private_file, write_runtime_settings,
};
use crate::runtime::{existing_llama, local_llm_model_path, sha256_file, LLM_MODEL_SHA256};

/// an optional Gemini API key, and the local-LLM endpoint into runtime.env so the
/// analysis worker picks the right strategy.
#[tauri::command]
pub(crate) fn set_highlight_strategy(
    _app: tauri::AppHandle,
    strategy: String,
    gemini_api_key: String,
    local_llm_base_url: String,
    local_llm_model: String,
) -> Result<String, String> {
    if !["heuristic", "semantic-gemini", "semantic-local"].contains(&strategy.as_str()) {
        return Err("INVALID_STRATEGY: choose heuristic, semantic-gemini or semantic-local".into());
    }
    if strategy == "semantic-local" {
        if !valid_local_llm_url(&local_llm_base_url) {
            return Err("LOCAL_LLM_URL_MUST_BE_LOOPBACK".into());
        }
        if !["qwen2.5-3b-instruct", "qwen2.5-3b-instruct-q4_k_m"]
            .contains(&local_llm_model.as_str())
        {
            return Err(
                "LOCAL_LLM_MODEL_UNSUPPORTED: only qwen2.5-3b-instruct is currently available"
                    .into(),
            );
        }
        let model_path = local_llm_model_path(&_app, &local_llm_model);
        let model_ready = model_path.as_ref().is_some_and(|path| {
            path.is_file() && sha256_file(path).is_ok_and(|sha| sha == LLM_MODEL_SHA256)
        });
        if existing_llama(cache_dir().join("llama/runtime")).is_none() || !model_ready {
            return Err(
                "LOCAL_LLM_SETUP_REQUIRED: install the local runtime and model in Settings first"
                    .into(),
            );
        }
    }
    let file = runtime_env();
    let command = file
        .get("WHISPER_COMMAND")
        .cloned()
        .filter(|c| !c.is_empty() && whisper_usable(c))
        .unwrap_or_else(whisper_command);
    let model = file
        .get("WHISPER_MODEL")
        .cloned()
        .filter(|c| !c.is_empty())
        .unwrap_or_else(|| "tiny".into());
    let device = file
        .get("WHISPER_DEVICE")
        .cloned()
        .filter(|c| !c.is_empty())
        .unwrap_or_else(|| "cpu".into());
    let fp16 = file
        .get("WHISPER_FP16")
        .cloned()
        .filter(|c| !c.is_empty())
        .unwrap_or_else(|| "False".into());
    // Keep previously stored values when switching strategies so the user's key/url are not lost.
    let stored_key = file.get("GEMINI_API_KEY").cloned().unwrap_or_default();
    // An empty key (or the UI's mask placeholder) means "keep the stored one":
    // the real key is never sent back to the UI, so echoing the mask used to
    // overwrite the stored key with "••••••••".
    let key = if strategy == "semantic-gemini" {
        if gemini_api_key.trim().is_empty() || gemini_api_key.trim() == "••••••••" {
            stored_key
        } else {
            gemini_api_key.clone()
        }
    } else {
        stored_key
    };
    let llama_url = if strategy == "semantic-local" {
        local_llm_base_url.clone()
    } else {
        file.get("LOCAL_LLM_BASE_URL")
            .cloned()
            .filter(|c| !c.is_empty())
            .unwrap_or_else(|| "http://127.0.0.1:8080".into())
    };
    let llama_model = if strategy == "semantic-local" {
        local_llm_model.clone()
    } else {
        file.get("LOCAL_LLM_MODEL")
            .cloned()
            .filter(|c| !c.is_empty())
            .unwrap_or_else(|| "qwen2.5-3b-instruct".into())
    };
    if strategy == "semantic-gemini" && key.trim().is_empty() {
        return Err("SEMANTIC_REQUIRES_GEMINI_API_KEY".into());
    }
    if strategy == "semantic-local" && llama_url.trim().is_empty() {
        return Err("LOCAL_LLM_REQUIRES_BASE_URL".into());
    }
    let mut settings = file;
    settings.insert("WHISPER_COMMAND".into(), command);
    settings.insert("WHISPER_MODEL".into(), model.clone());
    settings.insert("WHISPER_DEVICE".into(), device);
    settings.insert("WHISPER_FP16".into(), fp16);
    settings.insert("HIGHLIGHT_STRATEGY".into(), strategy.clone());
    settings.insert("GEMINI_API_KEY".into(), key);
    settings.insert("LOCAL_LLM_BASE_URL".into(), llama_url);
    settings.insert("LOCAL_LLM_MODEL".into(), llama_model);
    write_runtime_settings(&settings)?;
    Ok(strategy)
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AiProvider {
    id: String,
    name: String,
    base_url: String,
    #[serde(default)]
    api_key: String,
    model: String,
    updated_at: String,
}

#[derive(Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AiProviderStore {
    version: u32,
    active_provider_id: Option<String>,
    providers: Vec<AiProvider>,
}

pub(crate) fn write_ai_provider_store(store: &AiProviderStore) -> Result<(), String> {
    write_private_file(
        &data_dir().join("providers.json"),
        &serde_json::to_vec_pretty(store).map_err(|e| e.to_string())?,
    )
    .map_err(|e| format!("PROVIDERS_WRITE:{e}"))
}

pub(crate) fn flatten_active_provider(env: &mut HashMap<String, String>, store: &AiProviderStore) {
    if let Some(p) = store
        .providers
        .iter()
        .find(|p| Some(p.id.as_str()) == store.active_provider_id.as_deref())
    {
        for (k, v) in [
            ("ACTIVE_PROVIDER_ID", &p.id),
            ("ACTIVE_PROVIDER_NAME", &p.name),
            ("ACTIVE_PROVIDER_BASE_URL", &p.base_url),
            ("ACTIVE_PROVIDER_API_KEY", &p.api_key),
            ("ACTIVE_PROVIDER_MODEL", &p.model),
        ] {
            env.insert(k.into(), v.clone());
        }
        env.insert("HIGHLIGHT_STRATEGY".into(), "semantic-openai".into());
    } else {
        for k in [
            "ACTIVE_PROVIDER_ID",
            "ACTIVE_PROVIDER_NAME",
            "ACTIVE_PROVIDER_BASE_URL",
            "ACTIVE_PROVIDER_API_KEY",
            "ACTIVE_PROVIDER_MODEL",
        ] {
            env.remove(k);
        }
        if env.get("HIGHLIGHT_STRATEGY").map(String::as_str) == Some("semantic-openai") {
            env.insert("HIGHLIGHT_STRATEGY".into(), "heuristic".into());
        }
    }
}

pub(crate) fn read_ai_provider_store() -> Result<AiProviderStore, String> {
    let path = data_dir().join("providers.json");
    if path.exists() {
        return serde_json::from_slice(
            &std::fs::read(path).map_err(|e| format!("PROVIDERS_READ:{e}"))?,
        )
        .map_err(|e| format!("PROVIDERS_PARSE:{e}"));
    }
    let env = runtime_env();
    if env.get("HIGHLIGHT_STRATEGY").map(String::as_str) == Some("semantic-gemini") {
        if let Some(key) = env.get("GEMINI_API_KEY").filter(|v| !v.trim().is_empty()) {
            let p = AiProvider {
                id: "legacy-gemini".into(),
                name: "Gemini".into(),
                base_url: "https://generativelanguage.googleapis.com/v1beta/openai".into(),
                api_key: key.clone(),
                model: "gemini-2.0-flash".into(),
                updated_at: "legacy-migration".into(),
            };
            let store = AiProviderStore {
                version: 1,
                active_provider_id: Some(p.id.clone()),
                providers: vec![p],
            };
            write_ai_provider_store(&store)?;
            let mut updated = env;
            flatten_active_provider(&mut updated, &store);
            write_runtime_settings(&updated)?;
            return Ok(store);
        }
    }
    Ok(AiProviderStore {
        version: 1,
        ..Default::default()
    })
}

#[tauri::command]
pub(crate) fn list_ai_providers() -> Result<serde_json::Value, String> {
    let s = read_ai_provider_store()?;
    let providers:Vec<_>=s.providers.iter().map(|p|serde_json::json!({"id":p.id,"name":p.name,"baseUrl":p.base_url,"model":p.model,"hasKey":!p.api_key.is_empty()})).collect();
    Ok(serde_json::json!({"providers":providers,"activeProviderId":s.active_provider_id}))
}

#[tauri::command]
pub(crate) fn save_ai_provider(
    id: Option<String>,
    name: String,
    base_url: String,
    api_key: String,
    model: String,
) -> Result<String, String> {
    let name = name.trim().to_string();
    let Some(base_url) = valid_provider_url(&base_url) else {
        return Err("INVALID_PROVIDER_URL".into());
    };
    let model = model.trim().to_string();
    if name.is_empty() || model.is_empty() || api_key.contains(['\n', '\r']) {
        return Err("INVALID_PROVIDER_FIELDS".into());
    }
    let mut s = read_ai_provider_store()?;
    let id = id.filter(|v| !v.trim().is_empty());
    let pid = id.clone().unwrap_or_else(|| {
        format!(
            "provider-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        )
    });
    if let Some(p) = s.providers.iter_mut().find(|p| Some(&p.id) == id.as_ref()) {
        p.name = name;
        p.base_url = base_url;
        if !api_key.trim().is_empty() {
            p.api_key = api_key;
        }
        p.model = model;
        p.updated_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
            .to_string();
    } else {
        if api_key.trim().len() < 10 {
            return Err("PROVIDER_API_KEY_REQUIRED".into());
        }
        s.providers.push(AiProvider {
            id: pid.clone(),
            name,
            base_url,
            api_key,
            model,
            updated_at: "now".into(),
        });
    }
    write_ai_provider_store(&s)?;
    if s.active_provider_id.as_deref() == Some(&pid) {
        let mut env = runtime_env();
        flatten_active_provider(&mut env, &s);
        write_runtime_settings(&env)?;
    }
    Ok(pid)
}

#[tauri::command]
pub(crate) fn activate_ai_provider(id: String) -> Result<String, String> {
    let mut s = read_ai_provider_store()?;
    if !s.providers.iter().any(|p| p.id == id) {
        return Err("PROVIDER_NOT_FOUND".into());
    }
    s.active_provider_id = Some(id.clone());
    write_ai_provider_store(&s)?;
    let mut env = runtime_env();
    flatten_active_provider(&mut env, &s);
    write_runtime_settings(&env)?;
    Ok(id)
}

#[tauri::command]
pub(crate) fn delete_ai_provider(id: String) -> Result<(), String> {
    let mut s = read_ai_provider_store()?;
    s.providers.retain(|p| p.id != id);
    if s.active_provider_id.as_deref() == Some(&id) {
        s.active_provider_id = None;
    }
    write_ai_provider_store(&s)?;
    let mut env = runtime_env();
    flatten_active_provider(&mut env, &s);
    write_runtime_settings(&env)
}

/// Reports the currently stored highlight strategy plus key/url presence flags
/// (the key and url themselves are never returned to the UI).
#[tauri::command]
pub(crate) fn highlight_strategy_status(_app: tauri::AppHandle) -> serde_json::Value {
    let file = runtime_env();
    let strategy = file
        .get("HIGHLIGHT_STRATEGY")
        .cloned()
        .filter(|c| !c.is_empty())
        .unwrap_or_else(|| "heuristic".into());
    let has_key = file
        .get("GEMINI_API_KEY")
        .cloned()
        .map(|k| !k.trim().is_empty())
        .unwrap_or(false);
    let has_local_url = file
        .get("LOCAL_LLM_BASE_URL")
        .cloned()
        .map(|k| !k.trim().is_empty())
        .unwrap_or(false);
    serde_json::json!({ "strategy": strategy, "hasKey": has_key, "hasLocalUrl": has_local_url })
}


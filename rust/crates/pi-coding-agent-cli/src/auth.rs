use pi_ai::get_env_api_key;
use pi_coding_agent_core::AuthSource;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

#[derive(Clone)]
pub struct OverlayAuthSource {
    base: Arc<dyn AuthSource>,
    runtime_api_keys: Arc<Mutex<HashMap<String, String>>>,
}

impl OverlayAuthSource {
    pub fn new(base: Arc<dyn AuthSource>) -> Self {
        Self {
            base,
            runtime_api_keys: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn set_runtime_api_key(&self, provider: impl Into<String>, api_key: impl Into<String>) {
        self.runtime_api_keys
            .lock()
            .unwrap()
            .insert(provider.into(), api_key.into());
    }
}

impl AuthSource for OverlayAuthSource {
    fn has_auth(&self, provider: &str) -> bool {
        self.runtime_api_keys.lock().unwrap().contains_key(provider) || self.base.has_auth(provider)
    }

    fn get_api_key(&self, provider: &str) -> Option<String> {
        self.runtime_api_keys
            .lock()
            .unwrap()
            .get(provider)
            .cloned()
            .or_else(|| self.base.get_api_key(provider))
    }

    fn get_api_key_for_request<'a>(
        &'a self,
        provider: &'a str,
    ) -> pi_coding_agent_core::AuthApiKeyFuture<'a> {
        Box::pin(async move {
            let runtime_key = self.runtime_api_keys.lock().unwrap().get(provider).cloned();
            match runtime_key {
                Some(runtime_key) => Some(runtime_key),
                None => self.base.get_api_key_for_request(provider).await,
            }
        })
    }
}

#[derive(Debug, Default, Clone)]
pub struct EnvAuthSource;

impl EnvAuthSource {
    pub fn new() -> Self {
        Self
    }
}

impl AuthSource for EnvAuthSource {
    fn has_auth(&self, provider: &str) -> bool {
        self.get_api_key(provider).is_some()
    }

    fn get_api_key(&self, provider: &str) -> Option<String> {
        get_env_api_key(provider)
    }
}

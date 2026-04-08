use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

pub trait AuthSource: Send + Sync {
    fn has_auth(&self, provider: &str) -> bool;
    fn get_api_key(&self, provider: &str) -> Option<String>;
}

#[derive(Default, Clone)]
pub struct MemoryAuthStorage {
    api_keys: Arc<Mutex<HashMap<String, String>>>,
}

impl MemoryAuthStorage {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_api_keys(
        api_keys: impl IntoIterator<Item = (impl Into<String>, impl Into<String>)>,
    ) -> Self {
        let storage = Self::default();
        {
            let mut guard = storage.api_keys.lock().unwrap();
            for (provider, api_key) in api_keys {
                guard.insert(provider.into(), api_key.into());
            }
        }
        storage
    }

    pub fn set_api_key(&self, provider: impl Into<String>, api_key: impl Into<String>) {
        self.api_keys
            .lock()
            .unwrap()
            .insert(provider.into(), api_key.into());
    }

    pub fn remove_api_key(&self, provider: &str) {
        self.api_keys.lock().unwrap().remove(provider);
    }
}

impl AuthSource for MemoryAuthStorage {
    fn has_auth(&self, provider: &str) -> bool {
        self.api_keys.lock().unwrap().contains_key(provider)
    }

    fn get_api_key(&self, provider: &str) -> Option<String> {
        self.api_keys.lock().unwrap().get(provider).cloned()
    }
}

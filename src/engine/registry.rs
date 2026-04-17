use std::collections::HashMap;
use std::sync::Arc;

use crate::engine::backend::{InferenceBackend, ModelInfo};

pub struct BackendRegistry {
    backends: HashMap<String, Arc<dyn InferenceBackend>>,
}

impl BackendRegistry {
    pub fn new() -> Self {
        Self {
            backends: HashMap::new(),
        }
    }

    pub fn insert(&mut self, id: String, backend: Arc<dyn InferenceBackend>) {
        self.backends.insert(id, backend);
    }

    pub fn get(&self, model_id: &str) -> Option<Arc<dyn InferenceBackend>> {
        self.backends.get(model_id).cloned()
    }

    pub fn ids(&self) -> Vec<String> {
        let mut ids: Vec<String> = self.backends.keys().cloned().collect();
        ids.sort();
        ids
    }

    pub fn model_infos(&self) -> Vec<ModelInfo> {
        let mut infos: Vec<ModelInfo> = self.backends.values().map(|b| b.model_info()).collect();
        infos.sort_by(|a, b| a.id.cmp(&b.id));
        infos
    }

    pub fn len(&self) -> usize {
        self.backends.len()
    }

    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.backends.is_empty()
    }

    /// All registered backends, used for coordinated shutdown. Cloud
    /// backends keep the default no-op `shutdown`, so it's safe to call on
    /// every entry including routes.
    pub fn backends(&self) -> Vec<Arc<dyn InferenceBackend>> {
        self.backends.values().cloned().collect()
    }
}

impl Default for BackendRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl crate::engine::backend::Evictor for BackendRegistry {
    async fn unload(&self, model_id: &str) -> Result<(), crate::error::EngineError> {
        let backend = self.get(model_id).ok_or_else(|| {
            crate::error::EngineError::ModelNotFound(model_id.to_string())
        })?;
        backend.unload().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::testing::MockBackend;

    fn mock(id: &str) -> Arc<dyn InferenceBackend> {
        Arc::new(MockBackend::succeeding(id, "hi"))
    }

    #[test]
    fn test_registry_get_returns_backend_for_known_id() {
        let mut registry = BackendRegistry::new();
        registry.insert("qwen3-8b".to_string(), mock("qwen3-8b"));
        let backend = registry.get("qwen3-8b");
        assert!(backend.is_some());
        assert_eq!(backend.unwrap().model_info().id, "qwen3-8b");
    }

    #[test]
    fn test_registry_get_returns_none_for_unknown_id() {
        let registry = BackendRegistry::new();
        assert!(registry.get("does-not-exist").is_none());
    }

    #[test]
    fn test_registry_ids_returns_sorted_list() {
        let mut registry = BackendRegistry::new();
        registry.insert("qwen3-8b".to_string(), mock("qwen3-8b"));
        registry.insert("codellama-13b".to_string(), mock("codellama-13b"));
        let ids = registry.ids();
        assert_eq!(ids, vec!["codellama-13b", "qwen3-8b"]);
    }

    #[test]
    fn test_registry_model_infos_returns_one_per_backend() {
        let mut registry = BackendRegistry::new();
        registry.insert("qwen3-8b".to_string(), mock("qwen3-8b"));
        registry.insert("codellama-13b".to_string(), mock("codellama-13b"));
        let infos = registry.model_infos();
        assert_eq!(infos.len(), 2);
        assert_eq!(infos[0].id, "codellama-13b");
        assert_eq!(infos[1].id, "qwen3-8b");
    }

    #[test]
    fn test_empty_registry_is_empty() {
        let registry = BackendRegistry::new();
        assert!(registry.is_empty());
        assert_eq!(registry.len(), 0);
    }

    #[tokio::test]
    async fn test_evictor_unload_known_model_returns_ok() {
        use crate::engine::backend::Evictor;
        let mut reg = BackendRegistry::new();
        reg.insert("m".into(), mock("m"));
        let reg = Arc::new(reg);
        // Default no-op unload returns Ok.
        reg.unload("m").await.unwrap();
    }

    #[tokio::test]
    async fn test_evictor_unload_unknown_model_returns_not_found() {
        use crate::engine::backend::Evictor;
        let reg = Arc::new(BackendRegistry::new());
        let err = reg.unload("nope").await.unwrap_err();
        assert!(matches!(err, crate::error::EngineError::ModelNotFound(_)));
    }
}

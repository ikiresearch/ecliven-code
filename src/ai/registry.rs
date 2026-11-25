use std::sync::Arc;

use crate::ai::anthropic::AnthropicProvider;
use crate::ai::ollama::OllamaProvider;
use crate::ai::openai::OpenAiCompatibleProvider;
use crate::ai::{AiError, LlmProvider};
use crate::config::{AppConfig, ProviderKind};

pub fn build_provider(config: &AppConfig) -> Result<Arc<dyn LlmProvider>, AiError> {
    let kind = config.active_provider;
    let settings = config.active_settings();

    match kind {
        ProviderKind::Anthropic => {
            let api_key = settings.api_key.clone().ok_or(AiError::MissingApiKey {
                provider: "anthropic",
                env_var: "ANTHROPIC_API_KEY",
            })?;
            Ok(Arc::new(AnthropicProvider::new(
                api_key,
                settings.base_url.clone(),
                settings.model.clone(),
            )))
        }
        ProviderKind::OpenAi => {
            let api_key = settings.api_key.clone().ok_or(AiError::MissingApiKey {
                provider: "openai",
                env_var: "OPENAI_API_KEY",
            })?;
            Ok(Arc::new(OpenAiCompatibleProvider::new(
                "openai",
                api_key,
                settings.base_url.clone(),
                settings.model.clone(),
                false,
            )))
        }
        ProviderKind::OpenRouter => {
            let api_key = settings.api_key.clone().ok_or(AiError::MissingApiKey {
                provider: "openrouter",
                env_var: "OPENROUTER_API_KEY",
            })?;
            Ok(Arc::new(OpenAiCompatibleProvider::new(
                "openrouter",
                api_key,
                settings.base_url.clone(),
                settings.model.clone(),
                true,
            )))
        }
        ProviderKind::Ollama => Ok(Arc::new(OllamaProvider::new(
            settings.base_url.clone(),
            settings.model.clone(),
        ))),
    }
}

pub const DEFAULT_ANTHROPIC_MODEL: &str = "claude-sonnet-4-20250514";
pub const DEFAULT_OPENAI_MODEL: &str = "gpt-4.1";
pub const DEFAULT_OPENROUTER_MODEL: &str = "anthropic/claude-sonnet-4";
pub const DEFAULT_OLLAMA_MODEL: &str = "qwen2.5-coder:14b";
pub const DEFAULT_CLI_MODEL: &str = "default";

pub const DEFAULT_ANTHROPIC_BASE_URL: &str = "https://api.anthropic.com";
pub const DEFAULT_OPENAI_BASE_URL: &str = "https://api.openai.com/v1";
pub const DEFAULT_OPENROUTER_BASE_URL: &str = "https://openrouter.ai/api/v1";
pub const DEFAULT_OLLAMA_BASE_URL: &str = "http://localhost:11434";

pub const DEFAULT_MAX_TOKENS: u32 = 4096;
pub const DEFAULT_TEMPERATURE: f32 = 0.2;
pub const DEFAULT_MAX_TOOL_ITERATIONS: usize = 15;
pub const DEFAULT_SHELL_TIMEOUT_SECS: u64 = 30;
pub const DEFAULT_AUTO_COMPACT_PERCENT: u8 = 80;

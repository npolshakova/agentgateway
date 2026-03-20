pub mod bedrock;
pub mod completions;
pub mod count_tokens;
pub mod detect;
pub mod embeddings;
pub mod messages;
pub mod responses;
pub mod vertex;

use agent_core::prelude::Strng;
use agent_core::strng;

use crate::apply;
use crate::llm::{AIError, LLMRequest, LLMResponse};
use crate::serdes::schema;

/// ResponseType is an abstraction over provider/endpoint specific response formats that enables
/// uniform policy enforcement and observability
pub trait ResponseType: Send + Sync {
	fn to_llm_response(&self, include_completion_in_log: bool) -> LLMResponse;
	fn to_webhook_choices(&self) -> Vec<crate::llm::policy::webhook::ResponseChoice>;
	fn set_webhook_choices(
		&mut self,
		resp: Vec<crate::llm::policy::webhook::ResponseChoice>,
	) -> anyhow::Result<()>;
	fn serialize(&self) -> serde_json::Result<Vec<u8>>;
}

/// RequestType is an abstraction over provider/endpoint specific request formats that enables
/// uniform policy enforcement and observability
pub trait RequestType: Send + Sync {
	fn supports_model(&self) -> bool {
		true
	}
	fn model(&mut self) -> &mut Option<String>;
	fn prepend_prompts(&mut self, prompts: Vec<SimpleChatCompletionMessage>);
	fn append_prompts(&mut self, prompts: Vec<SimpleChatCompletionMessage>);
	fn to_llm_request(&self, provider: Strng, tokenize: bool) -> Result<LLMRequest, AIError>;
	fn get_messages(&self) -> Vec<SimpleChatCompletionMessage>;
	fn set_messages(&mut self, messages: Vec<SimpleChatCompletionMessage>);

	fn to_openai(&self) -> Result<Vec<u8>, AIError> {
		Err(AIError::UnsupportedConversion(strng::literal!("openai")))
	}

	fn to_anthropic(&self) -> Result<Vec<u8>, AIError> {
		Err(AIError::UnsupportedConversion(strng::literal!("anthropic")))
	}

	fn to_bedrock(
		&self,
		_provider: &crate::llm::bedrock::Provider,
		_headers: Option<&::http::HeaderMap>,
		_prompt_caching: Option<&crate::llm::policy::PromptCachingConfig>,
	) -> Result<Vec<u8>, AIError> {
		Err(AIError::UnsupportedConversion(strng::literal!("bedrock")))
	}

	fn to_bedrock_token_count(&self, _headers: &::http::HeaderMap) -> Result<Vec<u8>, AIError> {
		Err(AIError::UnsupportedConversion(strng::literal!(
			"bedrock token count"
		)))
	}

	fn to_vertex(&self, _provider: &crate::llm::vertex::Provider) -> Result<Vec<u8>, AIError> {
		Err(AIError::UnsupportedConversion(strng::literal!("vertex")))
	}

	/// Merge all system/developer messages into a single system message at index 0.
	///
	/// Gemini's OpenAI-compatible endpoint maps system messages to the native
	/// `system_instruction` field, which only accepts a **single** system message
	/// placed before the first conversational turn. When the request contains
	/// multiple system messages — or system messages that appear *after* user
	/// messages — the Gemini shim silently drops all but the first, causing
	/// significant prompt loss.
	///
	/// This method collects every system/developer message (regardless of
	/// position), concatenates their text, and re-inserts a single system message
	/// at the front of the messages array so that the full system context reaches
	/// the model.
	///
	/// The default implementation is a no-op; only formats that carry an inline
	/// messages array (i.e. chat completions) override it.
	fn consolidate_system_messages(&mut self) {}
}

/// SimpleChatCompletionMessage is a simplified chat message
#[apply(schema!)]
#[derive(Eq, PartialEq, cel::DynamicType)]
pub struct SimpleChatCompletionMessage {
	pub role: Strng,
	pub content: Strng,
}

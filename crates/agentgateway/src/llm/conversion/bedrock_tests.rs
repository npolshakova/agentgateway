use agent_core::strng;
use http::HeaderMap;
use serde_json::json;

use super::*;
use crate::llm::bedrock::Provider;
use crate::llm::types;

#[test]
fn test_extract_beta_headers_variants() {
	let headers = HeaderMap::new();
	assert!(helpers::extract_beta_headers(&headers).unwrap().is_none());

	let mut headers = HeaderMap::new();
	headers.insert(
		"anthropic-beta",
		"prompt-caching-2024-07-31".parse().unwrap(),
	);
	assert_eq!(
		helpers::extract_beta_headers(&headers).unwrap().unwrap(),
		vec![json!("prompt-caching-2024-07-31")]
	);

	let mut headers = HeaderMap::new();
	headers.insert(
		"anthropic-beta",
		"cache-control-2024-08-15,computer-use-2024-10-22"
			.parse()
			.unwrap(),
	);
	assert_eq!(
		helpers::extract_beta_headers(&headers).unwrap().unwrap(),
		vec![
			json!("cache-control-2024-08-15"),
			json!("computer-use-2024-10-22"),
		]
	);

	let mut headers = HeaderMap::new();
	headers.insert(
		"anthropic-beta",
		" cache-control-2024-08-15 , computer-use-2024-10-22 "
			.parse()
			.unwrap(),
	);
	assert_eq!(
		helpers::extract_beta_headers(&headers).unwrap().unwrap(),
		vec![
			json!("cache-control-2024-08-15"),
			json!("computer-use-2024-10-22"),
		]
	);

	let mut headers = HeaderMap::new();
	headers.append(
		"anthropic-beta",
		"cache-control-2024-08-15".parse().unwrap(),
	);
	headers.append("anthropic-beta", "computer-use-2024-10-22".parse().unwrap());
	let mut beta_features = helpers::extract_beta_headers(&headers)
		.unwrap()
		.unwrap()
		.into_iter()
		.map(|v| v.as_str().unwrap().to_string())
		.collect::<Vec<_>>();
	beta_features.sort();
	assert_eq!(
		beta_features,
		vec![
			"cache-control-2024-08-15".to_string(),
			"computer-use-2024-10-22".to_string(),
		]
	);
}

#[test]
fn test_metadata_from_header() {
	let provider = Provider {
		model: None,
		region: strng::new("us-east-1"),
		guardrail_identifier: None,
		guardrail_version: None,
	};

	// Simulate transformation CEL setting x-bedrock-metadata header
	let mut headers = HeaderMap::new();
	headers.insert(
		"x-bedrock-metadata",
		r#"{"user_id": "user123", "department": "engineering"}"#
			.parse()
			.unwrap(),
	);

	let req = messages::typed::Request {
		model: "anthropic.claude-3-sonnet".to_string(),
		messages: vec![messages::typed::Message {
			role: messages::typed::Role::User,
			content: vec![messages::typed::ContentBlock::Text(
				messages::typed::ContentTextBlock {
					text: "Hello".to_string(),
					citations: None,
					cache_control: None,
				},
			)],
		}],
		max_tokens: 100,
		metadata: None,
		system: None,
		stop_sequences: vec![],
		stream: false,
		temperature: None,
		top_k: None,
		top_p: None,
		tools: None,
		tool_choice: None,
		thinking: None,
	};

	let out = super::from_messages::translate_internal(req, &provider, Some(&headers));
	let metadata = out.request_metadata.unwrap();

	assert_eq!(metadata.get("user_id"), Some(&"user123".to_string()));
	assert_eq!(metadata.get("department"), Some(&"engineering".to_string()));
}

#[test]
fn test_metadata_from_completions_metadata_field() {
	let provider = Provider {
		model: None,
		region: strng::new("us-east-1"),
		guardrail_identifier: None,
		guardrail_version: None,
	};

	// OpenAI-style request metadata (agentgateway uses this to carry request-scoped guardrail knobs)
	let req = types::completions::typed::Request {
		model: Some("anthropic.claude-3-sonnet".to_string()),
		messages: vec![types::completions::typed::RequestMessage::User(
			types::completions::typed::RequestUserMessage {
				content: types::completions::typed::RequestUserMessageContent::Text("Hello".to_string()),
				name: None,
			},
		)],
		stream: None,
		temperature: None,
		top_p: None,
		max_completion_tokens: Some(16),
		stop: None,
		tools: None,
		tool_choice: None,
		parallel_tool_calls: None,
		user: Some("user456".to_string()),
		vendor_extensions: Default::default(),
		frequency_penalty: None,
		logit_bias: None,
		logprobs: None,
		top_logprobs: None,
		n: None,
		modalities: None,
		prediction: None,
		audio: None,
		presence_penalty: None,
		response_format: None,
		seed: None,
		#[allow(deprecated)]
		function_call: None,
		#[allow(deprecated)]
		functions: None,
		metadata: Some(json!({
			"user_id": "user123",
			"department": "engineering",
			// Non-string values should be ignored by the Bedrock metadata bridge
			"nonstr": 123
		})),
		#[allow(deprecated)]
		max_tokens: None,
		service_tier: None,
		web_search_options: None,
		stream_options: None,
		store: None,
		reasoning_effort: None,
	};

	let out = super::from_completions::translate_internal(
		req,
		"anthropic.claude-3-sonnet".to_string(),
		&provider,
		None,
		None,
	);
	let md = out.request_metadata.unwrap();

	// `metadata.user_id` should win over the `user`-derived value.
	assert_eq!(md.get("user_id"), Some(&"user123".to_string()));
	assert_eq!(md.get("department"), Some(&"engineering".to_string()));
	assert!(!md.contains_key("nonstr"));
}

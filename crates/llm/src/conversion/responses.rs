use std::time::Instant;

use agent_core::strng;
use axum_core::body::Body;
use serde::Deserialize;

use crate::types::detect;
use crate::{StreamingUsageGuard, parse, types};

#[derive(Debug, Clone)]
struct StreamResponse {
	typed: Option<types::responses::typed::ResponseStreamEvent>,
	raw: detect::StreamResponse,
}

impl<'de> Deserialize<'de> for StreamResponse {
	fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
	where
		D: serde::Deserializer<'de>,
	{
		let value = serde_json::Value::deserialize(deserializer)?;
		let typed = serde_json::from_value(value.clone()).ok();
		Ok(Self {
			typed,
			raw: detect::StreamResponse { rest: value },
		})
	}
}

pub fn passthrough_stream(
	b: Body,
	buffer_limit: usize,
	mut log: StreamingUsageGuard,
	include_completion_in_log: bool,
) -> Body {
	let mut saw_token = false;
	let mut completion = include_completion_in_log.then(String::new);
	parse::sse::json_passthrough::<StreamResponse>(b, buffer_limit, move |event| {
		let Some(Ok(event)) = event else {
			// Stream ended ([DONE]): flush completion if not already set via ResponseCompleted
			if event.is_none() {
				log.update(|r| {
					if let Some(c) = completion.take() {
						r.response.completion = Some(vec![c]);
					}
				});
			}
			return;
		};
		detect::amend_from_stream_response(&mut log, &event.raw);
		let Some(event) = event.typed else {
			return;
		};
		match event {
			types::responses::typed::ResponseStreamEvent::ResponseCreated(created) => {
				log.update(|r| {
					r.response.provider_model = Some(strng::new(&created.response.model));
					r.response.service_tier = created
						.response
						.service_tier
						.as_ref()
						.and_then(types::serialize_str);
					if let Some(usage) = &created.response.usage {
						r.response.input_tokens = Some(usage.input_tokens as u64);
						r.response.output_tokens = Some(usage.output_tokens as u64);
						r.response.total_tokens = Some(usage.total_tokens as u64);
						r.response.cached_input_tokens = Some(usage.input_tokens_details.cached_tokens as u64);
						r.response.reasoning_tokens = Some(usage.output_tokens_details.reasoning_tokens as u64);
					}
				});
			},
			types::responses::typed::ResponseStreamEvent::ResponseOutputTextDelta(ref delta) => {
				if !saw_token {
					saw_token = true;
					log.update(|r| {
						r.response.first_token = Some(Instant::now());
					});
				}
				if let Some(c) = completion.as_mut() {
					c.push_str(&delta.delta);
				}
			},
			types::responses::typed::ResponseStreamEvent::ResponseCompleted(completed) => {
				log.update(|r| {
					r.response.provider_model = Some(strng::new(&completed.response.model));
					r.response.service_tier = completed
						.response
						.service_tier
						.as_ref()
						.and_then(types::serialize_str);
					if let Some(usage) = &completed.response.usage {
						r.response.input_tokens = Some(usage.input_tokens as u64);
						r.response.output_tokens = Some(usage.output_tokens as u64);
						r.response.total_tokens = Some(usage.total_tokens as u64);
						r.response.cached_input_tokens = Some(usage.input_tokens_details.cached_tokens as u64);
						// TODO(GPT-5.6 explicit prompt caching): also record
						// `input_tokens_details.cache_write_tokens` once async-openai's
						// `InputTokenDetails` carries it (https://github.com/64bit/async-openai/pull/572).
						// Non-stream responses and the raw-detect fallback already capture it.
						r.response.reasoning_tokens = Some(usage.output_tokens_details.reasoning_tokens as u64);
					}
					if let Some(c) = completion.take() {
						r.response.completion = Some(vec![c]);
					}
				});
			},
			_ => {},
		}
	})
}

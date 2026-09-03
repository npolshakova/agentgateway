use bytes::Bytes;
use serde_json::{Value, json};

use super::to_responses;

fn translate(input: &[u8]) -> Value {
	let response = to_responses::translate_response(&Bytes::copy_from_slice(input), "input-model")
		.expect("Chat Completions response should translate");
	serde_json::from_slice(&response.serialize().expect("response should serialize"))
		.expect("translated response should be JSON")
}

#[test]
fn buffered_failed_response_preserves_error() {
	let mut input: Value =
		serde_json::from_slice(include_bytes!("../tests/response/completions/basic.json")).unwrap();
	input["choices"][0]["finish_reason"] = json!("content_filter");
	let input = serde_json::to_vec(&input).unwrap();

	let response = translate(&input);

	assert_eq!(response["status"], json!("failed"));
	assert_eq!(
		response["error"],
		json!({"code": "content_filter", "message": "Content filtered"})
	);
}

#[test]
fn buffered_reasoning_content_becomes_a_reasoning_item_before_the_message() {
	let mut input: Value =
		serde_json::from_slice(include_bytes!("../tests/response/completions/basic.json")).unwrap();
	input["choices"][0]["message"]["reasoning_content"] = json!("weighing the options");
	let input = serde_json::to_vec(&input).unwrap();

	let response = translate(&input);

	let output = response["output"].as_array().expect("output array");
	assert_eq!(output[0]["type"], json!("reasoning"));
	assert_eq!(output[0]["status"], json!("completed"));
	assert_eq!(output[0]["summary"][0]["type"], json!("summary_text"));
	assert_eq!(
		output[0]["summary"][0]["text"],
		json!("weighing the options")
	);
	assert_eq!(output[1]["type"], json!("message"));
}

#[test]
fn buffered_response_without_reasoning_has_no_reasoning_item() {
	let input = include_bytes!("../tests/response/completions/basic.json");
	let response = translate(input);
	let types: Vec<&Value> = response["output"]
		.as_array()
		.unwrap()
		.iter()
		.map(|o| &o["type"])
		.collect();
	assert!(!types.contains(&&json!("reasoning")));
}

/// Drive the streaming translator over an OpenAI-compatible SSE body and return
/// the emitted Responses events, parsed.
async fn translate_stream(chunks: &[Value]) -> Vec<Value> {
	use http_body_util::BodyExt;
	let mut sse = String::new();
	for c in chunks {
		sse.push_str("data: ");
		sse.push_str(&c.to_string());
		sse.push_str("\n\n");
	}
	sse.push_str("data: [DONE]\n\n");
	let body = to_responses::translate_stream(
		axum_core::body::Body::from(sse),
		1 << 20,
		crate::StreamingUsageGuard::default(),
		crate::LogContentFields::default(),
	);
	let bytes = body.collect().await.expect("stream body").to_bytes();
	String::from_utf8(bytes.to_vec())
		.expect("utf8")
		.lines()
		.filter_map(|l| l.strip_prefix("data: "))
		.map(|d| serde_json::from_str::<Value>(d).expect("event json"))
		.collect()
}

fn chunk(delta: Value, finish_reason: Option<&str>, usage: Option<Value>) -> Value {
	let mut c = json!({
		"id": "chatcmpl-1", "object": "chat.completion.chunk", "created": 1, "model": "glm-5.3",
		"choices": [{"index": 0, "delta": delta, "finish_reason": finish_reason}]
	});
	if let Some(u) = usage {
		c["usage"] = u;
	}
	c
}

#[tokio::test]
async fn stream_terminal_events_carry_the_finished_items() {
	let events = translate_stream(&[
		chunk(
			json!({"role": "assistant", "reasoning_content": "weigh"}),
			None,
			None,
		),
		chunk(json!({"reasoning_content": "ing"}), None, None),
		chunk(json!({"content": "o"}), None, None),
		chunk(json!({"content": "k"}), None, None),
		chunk(
			json!({}),
			Some("stop"),
			Some(json!({"prompt_tokens": 3, "completion_tokens": 5, "total_tokens": 8})),
		),
	])
	.await;
	let types: Vec<&str> = events.iter().map(|e| e["type"].as_str().unwrap()).collect();

	// The API's event order: created, in_progress, then the reasoning item (index
	// 0) streams before the message (index 1), and every "done" carries the whole.
	assert_eq!(types[0], "response.created");
	assert_eq!(types[1], "response.in_progress");
	assert_eq!(types[2], "response.output_item.added");
	assert_eq!(events[2]["item"]["type"], json!("reasoning"));
	assert_eq!(events[2]["output_index"], json!(0));
	assert!(types.contains(&"response.reasoning_summary_text.delta"));
	assert!(types.contains(&"response.output_text.done"));

	let text_done = events
		.iter()
		.find(|e| e["type"] == "response.output_text.done")
		.unwrap();
	assert_eq!(text_done["text"], json!("ok"));
	assert_eq!(text_done["output_index"], json!(1));

	let reasoning_done = events
		.iter()
		.find(|e| e["type"] == "response.reasoning_summary_text.done")
		.unwrap();
	assert_eq!(reasoning_done["text"], json!("weighing"));

	let completed = events.last().unwrap();
	assert_eq!(completed["type"], json!("response.completed"));
	let output = completed["response"]["output"].as_array().unwrap();
	assert_eq!(output.len(), 2);
	assert_eq!(output[0]["type"], json!("reasoning"));
	assert_eq!(output[0]["summary"][0]["text"], json!("weighing"));
	assert_eq!(output[1]["type"], json!("message"));
	assert_eq!(output[1]["content"][0]["text"], json!("ok"));
	assert_eq!(completed["response"]["usage"]["output_tokens"], json!(5));

	// sequence numbers are strictly increasing
	let seqs: Vec<u64> = events
		.iter()
		.map(|e| e["sequence_number"].as_u64().unwrap())
		.collect();
	assert!(seqs.windows(2).all(|w| w[1] == w[0] + 1));
}

#[tokio::test]
async fn stream_without_reasoning_puts_the_message_at_index_zero() {
	let events = translate_stream(&[
		chunk(json!({"role": "assistant", "content": ""}), None, None),
		chunk(json!({"content": "ok"}), None, None),
		chunk(
			json!({}),
			Some("length"),
			Some(json!({"prompt_tokens": 3, "completion_tokens": 1, "total_tokens": 4})),
		),
	])
	.await;
	let added = events
		.iter()
		.find(|e| e["type"] == "response.output_item.added")
		.unwrap();
	assert_eq!(added["item"]["type"], json!("message"));
	assert_eq!(added["output_index"], json!(0));

	let last = events.last().unwrap();
	assert_eq!(last["type"], json!("response.incomplete"));
	assert_eq!(
		last["response"]["incomplete_details"]["reason"],
		json!("max_tokens")
	);
	assert_eq!(
		last["response"]["output"][0]["content"][0]["text"],
		json!("ok")
	);
}

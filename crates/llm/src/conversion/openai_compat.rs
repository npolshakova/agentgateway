#[cfg(test)]
#[path = "openai_compat_tests.rs"]
mod tests;

pub mod from_responses {
	use types::completions::typed as completions;
	use types::responses::typed as responses;

	use crate::{AIError, json, types};

	/// Translate an OpenAI Responses request into an OpenAI-compatible chat completions request.
	pub fn translate(req: &types::responses::Request) -> Result<Vec<u8>, AIError> {
		let xlated = translate_request(req)?;
		serde_json::to_vec(&xlated).map_err(AIError::RequestMarshal)
	}

	pub fn translate_request(
		req: &types::responses::Request,
	) -> Result<types::completions::typed::Request, AIError> {
		let typed =
			json::convert::<_, responses::CreateResponse>(req).map_err(AIError::RequestMarshal)?;
		Ok(translate_internal(typed))
	}

	fn translate_internal(req: responses::CreateResponse) -> completions::Request {
		use responses::{
			EasyInputContent, InputContent, InputItem, InputMessage, InputParam, InputRole,
			InputTextContent, Item, MessageItem, OutputMessageContent, Role as ResponsesRole,
			TextResponseFormatConfiguration,
		};

		let mut messages: Vec<completions::RequestMessage> = Vec::new();

		if let Some(instructions) = &req.instructions {
			messages.push(completions::RequestMessage::Developer(
				completions::RequestDeveloperMessage {
					content: completions::RequestDeveloperMessageContent::Text(instructions.clone()),
					name: None,
				},
			));
		}

		let items = match &req.input {
			InputParam::Text(text) => vec![InputItem::from(InputMessage {
				content: vec![InputContent::InputText(InputTextContent {
					text: text.clone(),
					prompt_cache_breakpoint: None,
				})],
				role: InputRole::User,
				status: None,
			})],
			InputParam::Items(items) => items.clone(),
		};

		for item in items {
			match item {
				InputItem::EasyMessage(msg) => match msg.role {
					ResponsesRole::User => {
						let content = match msg.content {
							EasyInputContent::Text(text) => completions::RequestUserMessageContent::Text(text),
							EasyInputContent::ContentList(parts) => {
								completions::RequestUserMessageContent::Array(
									parts
										.into_iter()
										.filter_map(|part| match part {
											InputContent::InputText(text) => {
												Some(completions::RequestUserMessageContentPart::Text(
													completions::RequestMessageContentPartText {
														text: text.text,
														prompt_cache_breakpoint: text.prompt_cache_breakpoint,
													},
												))
											},
											_ => None,
										})
										.collect(),
								)
							},
						};
						messages.push(completions::RequestMessage::User(
							completions::RequestUserMessage {
								content,
								name: None,
							},
						));
					},
					ResponsesRole::Assistant => {
						let content = match msg.content {
							EasyInputContent::Text(text) => {
								completions::RequestAssistantMessageContent::Text(text)
							},
							EasyInputContent::ContentList(parts) => {
								completions::RequestAssistantMessageContent::Array(
									parts
										.into_iter()
										.filter_map(|part| match part {
											InputContent::InputText(text) => {
												Some(completions::RequestAssistantMessageContentPart::Text(
													completions::RequestMessageContentPartText {
														text: text.text,
														prompt_cache_breakpoint: text.prompt_cache_breakpoint,
													},
												))
											},
											_ => None,
										})
										.collect(),
								)
							},
						};
						messages.push(completions::RequestMessage::Assistant(
							completions::RequestAssistantMessage {
								content: Some(content),
								..Default::default()
							},
						));
					},
					ResponsesRole::System | ResponsesRole::Developer => {
						let content = match msg.content {
							EasyInputContent::Text(text) => {
								completions::RequestDeveloperMessageContent::Text(text)
							},
							EasyInputContent::ContentList(parts) => {
								completions::RequestDeveloperMessageContent::Array(
									parts
										.into_iter()
										.filter_map(|part| match part {
											InputContent::InputText(text) => {
												Some(completions::RequestDeveloperMessageContentPart::Text(
													completions::RequestMessageContentPartText {
														text: text.text,
														prompt_cache_breakpoint: text.prompt_cache_breakpoint,
													},
												))
											},
											_ => None,
										})
										.collect(),
								)
							},
						};
						messages.push(completions::RequestMessage::Developer(
							completions::RequestDeveloperMessage {
								content,
								name: None,
							},
						));
					},
				},
				InputItem::ItemReference(_) => continue,
				InputItem::Program(_) | InputItem::ProgramOutput(_) | InputItem::CompactionTrigger(_) => {
					tracing::debug!(
						"Skipping unsupported Responses input item for OpenAI-compatible chat completions"
					);
					continue;
				},
				InputItem::Item(item) => match item {
					Item::Message(msg_item) => match msg_item {
						MessageItem::Input(msg) => match msg.role {
							InputRole::User => {
								messages.push(completions::RequestMessage::User(
									completions::RequestUserMessage {
										content: completions::RequestUserMessageContent::Array(
											msg
												.content
												.into_iter()
												.filter_map(|content| match content {
													InputContent::InputText(text) => {
														Some(completions::RequestUserMessageContentPart::Text(
															completions::RequestMessageContentPartText {
																text: text.text,
																prompt_cache_breakpoint: text.prompt_cache_breakpoint,
															},
														))
													},
													_ => None,
												})
												.collect(),
										),
										name: None,
									},
								));
							},
							InputRole::System => {
								messages.push(completions::RequestMessage::System(
									completions::RequestSystemMessage {
										content: completions::RequestSystemMessageContent::Array(
											msg
												.content
												.into_iter()
												.filter_map(|content| match content {
													InputContent::InputText(text) => {
														Some(completions::RequestSystemMessageContentPart::Text(
															completions::RequestMessageContentPartText {
																text: text.text,
																prompt_cache_breakpoint: text.prompt_cache_breakpoint,
															},
														))
													},
													_ => None,
												})
												.collect(),
										),
										name: None,
									},
								));
							},
							InputRole::Developer => {
								messages.push(completions::RequestMessage::Developer(
									completions::RequestDeveloperMessage {
										content: completions::RequestDeveloperMessageContent::Array(
											msg
												.content
												.into_iter()
												.filter_map(|content| match content {
													InputContent::InputText(text) => {
														Some(completions::RequestDeveloperMessageContentPart::Text(
															completions::RequestMessageContentPartText {
																text: text.text,
																prompt_cache_breakpoint: text.prompt_cache_breakpoint,
															},
														))
													},
													_ => None,
												})
												.collect(),
										),
										name: None,
									},
								));
							},
						},
						MessageItem::Output(msg) => {
							let text = msg
								.content
								.iter()
								.filter_map(|c| match c {
									OutputMessageContent::OutputText(t) => Some(t.text.clone()),
									_ => None,
								})
								.collect::<Vec<_>>()
								.join("\n");

							messages.push(completions::RequestMessage::Assistant(
								completions::RequestAssistantMessage {
									content: if text.is_empty() {
										None
									} else {
										Some(completions::RequestAssistantMessageContent::Text(text))
									},
									..Default::default()
								},
							));
						},
					},
					Item::FunctionCall(call) => {
						let tool_call = completions::MessageToolCalls::Function(completions::MessageToolCall {
							id: call.call_id.clone(),
							function: completions::FunctionCall {
								name: call.name.clone(),
								arguments: call.arguments.clone(),
							},
						});
						if let Some(completions::RequestMessage::Assistant(message)) = messages.last_mut()
							&& let Some(tool_calls) = &mut message.tool_calls
						{
							tool_calls.push(tool_call);
						} else {
							messages.push(completions::RequestMessage::Assistant(
								completions::RequestAssistantMessage {
									tool_calls: Some(vec![tool_call]),
									..Default::default()
								},
							));
						}
					},
					Item::FunctionCallOutput(output) => {
						let output_text = match output.output {
							responses::FunctionCallOutput::Text(text) => text,
							responses::FunctionCallOutput::Content(parts) => parts
								.iter()
								.filter_map(|part| match part {
									InputContent::InputText(t) => Some(t.text.clone()),
									_ => None,
								})
								.collect::<Vec<_>>()
								.join("\n"),
						};
						messages.push(completions::RequestMessage::Tool(
							completions::RequestToolMessage {
								content: completions::RequestToolMessageContent::Text(output_text),
								tool_call_id: output.call_id,
							},
						));
					},
					Item::CustomToolCall(call) => {
						let arguments = serde_json::to_string(&call.input).unwrap_or_else(|_| "{}".to_string());
						let tool_call = completions::MessageToolCalls::Function(completions::MessageToolCall {
							id: call.id.clone(),
							function: completions::FunctionCall {
								name: call.name.clone(),
								arguments,
							},
						});
						if let Some(completions::RequestMessage::Assistant(message)) = messages.last_mut()
							&& let Some(tool_calls) = &mut message.tool_calls
						{
							tool_calls.push(tool_call);
						} else {
							messages.push(completions::RequestMessage::Assistant(
								completions::RequestAssistantMessage {
									tool_calls: Some(vec![tool_call]),
									..Default::default()
								},
							));
						}
					},
					Item::CustomToolCallOutput(output) => {
						let text = match &output.output {
							responses::CustomToolCallOutputOutput::Text(t) => t.clone(),
							_ => continue,
						};
						messages.push(completions::RequestMessage::Tool(
							completions::RequestToolMessage {
								content: completions::RequestToolMessageContent::Text(text),
								tool_call_id: output.id.clone().unwrap_or_default(),
							},
						));
					},
					_ => continue,
				},
			}
		}

		let tools: Option<Vec<completions::Tool>> = req.tools.as_ref().map(|tools| {
			tools
				.iter()
				.filter_map(|tool| match tool {
					responses::Tool::Function(func) => {
						Some(completions::Tool::Function(completions::FunctionTool {
							function: completions::FunctionObject {
								name: func.name.clone(),
								description: func.description.clone(),
								parameters: func.parameters.clone(),
								strict: func.strict,
							},
						}))
					},
					_ => None,
				})
				.collect()
		});

		let tool_choice = req.tool_choice.as_ref().and_then(|tc| {
			use responses::{ToolChoiceFunction, ToolChoiceOptions, ToolChoiceParam};
			match tc {
				ToolChoiceParam::Mode(ToolChoiceOptions::Auto) => Some(
					completions::ToolChoiceOption::Mode(completions::ToolChoiceOptions::Auto),
				),
				ToolChoiceParam::Mode(ToolChoiceOptions::Required) => Some(
					completions::ToolChoiceOption::Mode(completions::ToolChoiceOptions::Required),
				),
				ToolChoiceParam::Mode(ToolChoiceOptions::None) => Some(
					completions::ToolChoiceOption::Mode(completions::ToolChoiceOptions::None),
				),
				ToolChoiceParam::Function(ToolChoiceFunction { name }) => Some(
					completions::ToolChoiceOption::Function(completions::NamedToolChoice {
						function: completions::FunctionName { name: name.clone() },
					}),
				),
				ToolChoiceParam::Hosted(_)
				| ToolChoiceParam::AllowedTools(_)
				| ToolChoiceParam::Mcp(_)
				| ToolChoiceParam::Custom(_)
				| ToolChoiceParam::ProgrammaticToolCalling(_)
				| ToolChoiceParam::ApplyPatch
				| ToolChoiceParam::Shell => {
					tracing::warn!(
						"Unsupported tool choice for OpenAI-compatible chat completions: {:?}",
						tc
					);
					None
				},
			}
		});

		let reasoning_effort = req.reasoning.as_ref().and_then(|r| {
			r.effort.as_ref().and_then(|e| match e {
				responses::ReasoningEffort::Minimal => Some(completions::ReasoningEffort::Minimal),
				responses::ReasoningEffort::Low => Some(completions::ReasoningEffort::Low),
				responses::ReasoningEffort::Medium => Some(completions::ReasoningEffort::Medium),
				responses::ReasoningEffort::High => Some(completions::ReasoningEffort::High),
				responses::ReasoningEffort::Xhigh => Some(completions::ReasoningEffort::Xhigh),
				responses::ReasoningEffort::Max => Some(completions::ReasoningEffort::Max),
				responses::ReasoningEffort::None => None,
			})
		});

		let response_format = req.text.as_ref().and_then(|text| match &text.format {
			TextResponseFormatConfiguration::JsonSchema(json_schema) => {
				Some(completions::ResponseFormat::JsonSchema {
					json_schema: completions::ResponseFormatJsonSchema {
						description: json_schema.description.clone(),
						name: json_schema.name.clone(),
						schema: json_schema.schema.clone(),
						strict: json_schema.strict,
					},
				})
			},
			TextResponseFormatConfiguration::JsonObject => Some(completions::ResponseFormat::JsonObject),
			TextResponseFormatConfiguration::Text => None,
		});

		let stream = req.stream.unwrap_or(false);
		let stream_options = if stream {
			Some(completions::StreamOptions {
				include_usage: Some(true),
				include_obfuscation: None,
			})
		} else {
			None
		};

		#[allow(deprecated)]
		completions::Request {
			messages,
			tools,
			tool_choice,
			stream_options,
			reasoning_effort,
			response_format,
			stream: Some(stream),
			model: req.model.clone(),
			moderation: None,
			temperature: req.temperature,
			top_p: req.top_p,
			max_completion_tokens: req.max_output_tokens,
			parallel_tool_calls: req.parallel_tool_calls,
			vendor_extensions: completions::RequestVendorExtensions::default(),
			max_tokens: None,
			stop: None,
			user: None,
			frequency_penalty: None,
			presence_penalty: None,
			seed: None,
			store: None,
			metadata: None,
			logit_bias: None,
			logprobs: None,
			top_logprobs: None,
			n: None,
			modalities: None,
			prediction: None,
			audio: None,
			function_call: None,
			functions: None,
			service_tier: None,
			web_search_options: None,
		}
	}
}

pub mod to_responses {
	use std::collections::HashMap;
	use std::time::Instant;

	use agent_core::strng;
	use axum_core::body::Body;
	use bytes::Bytes;
	use rand::RngExt;
	use types::completions::typed as completions;
	use types::responses::typed as responses;

	use crate::parse::sse::SseJsonEvent;
	use crate::types::ResponseType;
	use crate::{AIError, StreamingUsageGuard, json, logged_response_parsing, parse, types};

	type LoggedToolCall = (Option<String>, Option<String>, String);
	type LoggedToolCalls = HashMap<u32, LoggedToolCall>;

	/// Translate an OpenAI-compatible chat completions response into an OpenAI Responses response.
	pub fn translate_response(bytes: &Bytes, model: &str) -> Result<Box<dyn ResponseType>, AIError> {
		let resp = serde_json::from_slice::<completions::Response>(bytes)
			.map_err(logged_response_parsing(bytes))?;
		let typed = translate_response_internal(resp, model);
		let passthrough =
			json::convert::<_, types::responses::Response>(&typed).map_err(AIError::ResponseParsing)?;
		Ok(Box::new(passthrough))
	}

	fn translate_response_internal(resp: completions::Response, model: &str) -> responses::Response {
		let response_id = format!("resp_{:016x}", rand::rng().random::<u64>());
		let response_builder = types::responses::ResponseBuilder::new(response_id, model.to_string());

		let choice = resp.choices.into_iter().next();

		let mut outputs: Vec<responses::OutputItem> = Vec::new();
		let mut text_parts: Vec<responses::OutputMessageContent> = Vec::new();
		let mut tool_calls: Vec<responses::OutputItem> = Vec::new();

		if let Some(choice) = &choice {
			if let Some(content) = &choice.message.content {
				text_parts.push(responses::OutputMessageContent::OutputText(
					responses::OutputTextContent {
						annotations: vec![],
						logprobs: None,
						text: content.clone(),
					},
				));
			}

			if let Some(tcs) = &choice.message.tool_calls {
				for tc in tcs {
					match tc {
						completions::MessageToolCalls::Function(f) => {
							tool_calls.push(responses::OutputItem::FunctionCall(
								responses::FunctionToolCall {
									arguments: f.function.arguments.clone(),
									call_id: f.id.clone(),
									name: f.function.name.clone(),
									caller: None,
									id: Some(f.id.clone()),
									status: Some(responses::OutputStatus::Completed),
									namespace: None,
								},
							));
						},
						completions::MessageToolCalls::Custom(_) => {},
					}
				}
			}
		}

		// A provider's `reasoning_content` (DeepSeek, z.ai, LiteLLM, our Bedrock/Anthropic
		// bridge) is the model's reasoning; the Responses API carries it as a
		// `reasoning` item with a summary part, listed BEFORE the message.
		if let Some(reasoning) = choice
			.as_ref()
			.and_then(|c| c.message.reasoning_content.as_deref())
			.filter(|r| !r.is_empty())
		{
			outputs.push(responses::OutputItem::Reasoning(responses::ReasoningItem {
				id: Some(format!("rs_{:016x}", rand::rng().random::<u64>())),
				summary: vec![responses::SummaryPart::SummaryText(
					responses::SummaryTextContent {
						text: reasoning.to_string(),
					},
				)],
				content: None,
				encrypted_content: None,
				status: Some(responses::OutputStatus::Completed),
			}));
		}

		if !text_parts.is_empty() {
			outputs.push(responses::OutputItem::Message(responses::OutputMessage {
				id: format!("msg_{:016x}", rand::rng().random::<u64>()),
				role: responses::AssistantRole::Assistant,
				phase: None,
				content: text_parts,
				status: responses::OutputStatus::Completed,
			}));
		}
		outputs.extend(tool_calls);

		let finish_reason = choice.as_ref().and_then(|c| c.finish_reason.as_ref());

		let status = match finish_reason {
			Some(completions::FinishReason::Stop) | None => responses::Status::Completed,
			Some(completions::FinishReason::Length) => responses::Status::Incomplete,
			Some(completions::FinishReason::ToolCalls)
			| Some(completions::FinishReason::FunctionCall) => responses::Status::Completed,
			Some(completions::FinishReason::ContentFilter) => responses::Status::Failed,
		};

		let incomplete_details = match finish_reason {
			Some(completions::FinishReason::Length) => Some(responses::IncompleteDetails {
				reason: "max_tokens".to_string(),
			}),
			_ => None,
		};

		let error = match finish_reason {
			Some(completions::FinishReason::ContentFilter) => Some(responses::ErrorObject {
				code: "content_filter".to_string(),
				message: "Content filtered".to_string(),
			}),
			_ => None,
		};

		let usage = resp.usage.map(|u| responses::ResponseUsage {
			input_tokens: u.prompt_tokens,
			output_tokens: usage_output_tokens(&u),
			total_tokens: u.total_tokens,
			input_tokens_details: responses::InputTokenDetails {
				cached_tokens: u
					.prompt_tokens_details
					.as_ref()
					.and_then(|d| d.cached_tokens)
					.unwrap_or(0) as u32,
				cache_write_tokens: u
					.prompt_tokens_details
					.as_ref()
					.and_then(|d| d.cache_write_tokens)
					.or(u.cache_creation_input_tokens)
					.map(|tokens| tokens as u32),
			},
			output_tokens_details: responses::OutputTokenDetails {
				reasoning_tokens: u
					.completion_tokens_details
					.as_ref()
					.and_then(|d| d.reasoning_tokens)
					.unwrap_or(0) as u32,
			},
		});

		let mut response = response_builder.response(status, usage, error, incomplete_details);
		response.output = outputs;
		response
	}

	/// What the stream translator accumulates so the terminal events can carry
	/// complete items. The Responses API's `response.completed` payload lists the
	/// finished output items and `response.output_text.done` carries the whole
	/// text — SDK clients (`get_final_response()`) read those rather than
	/// re-assembling deltas, so emitting them empty loses the answer.
	struct StreamState {
		sequence_number: u64,
		response_id: String,
		model: String,
		next_output_index: u32,
		/// The assistant message. Created lazily on the first text delta so a
		/// reasoning item can take the earlier output index, as the API orders them.
		message_item_id: String,
		message_index: Option<u32>,
		sent_content_part: bool,
		text: String,
		/// `reasoning_content` deltas → one reasoning item with one summary part.
		reasoning_item_id: String,
		reasoning_index: Option<u32>,
		reasoning: String,
		/// index in the upstream chunk → (item id, name, arguments so far, output index)
		tool_calls: HashMap<u32, (String, String, String, u32)>,
		pending_stop_reason: Option<completions::FinishReason>,
		pending_usage: Option<completions::Usage>,
		completion: Option<String>,
		logged_tool_calls: Option<LoggedToolCalls>,
	}

	impl StreamState {
		fn next_seq(&mut self) -> u64 {
			self.sequence_number += 1;
			self.sequence_number
		}

		fn take_output_index(&mut self) -> u32 {
			let idx = self.next_output_index;
			self.next_output_index += 1;
			idx
		}
	}

	pub fn translate_stream(
		b: Body,
		buffer_limit: usize,
		log: StreamingUsageGuard,
		log_content: crate::LogContentFields,
	) -> Body {
		use responses::{
			AssistantRole, FunctionToolCall, OutputContent, OutputItem, OutputMessage, OutputStatus,
			OutputTextContent, ReasoningItem, ResponseContentPartAddedEvent,
			ResponseFunctionCallArgumentsDeltaEvent, ResponseInProgressEvent,
			ResponseOutputItemAddedEvent, ResponseReasoningSummaryPartAddedEvent,
			ResponseReasoningSummaryTextDeltaEvent, ResponseStreamEvent, ResponseTextDeltaEvent,
			SummaryPart, SummaryTextContent,
		};

		let mut saw_token = false;
		let mut sent_created = false;
		let mut flushed = false;
		let mut st = StreamState {
			sequence_number: 0,
			response_id: format!("resp_{:016x}", rand::rng().random::<u64>()),
			model: String::new(),
			next_output_index: 0,
			message_item_id: format!("msg_{:016x}", rand::rng().random::<u64>()),
			message_index: None,
			sent_content_part: false,
			text: String::new(),
			reasoning_item_id: format!("rs_{:016x}", rand::rng().random::<u64>()),
			reasoning_index: None,
			reasoning: String::new(),
			tool_calls: HashMap::new(),
			pending_stop_reason: None,
			pending_usage: None,
			completion: log_content.completion.then(String::new),
			logged_tool_calls: log_content.tool_calls.then(HashMap::new),
		};

		parse::sse::json_transform_multi::<completions::StreamResponse, ResponseStreamEvent, _>(
			b,
			buffer_limit,
			move |evt| {
				let mut events: Vec<(&'static str, ResponseStreamEvent)> = Vec::new();

				match evt {
					SseJsonEvent::Eof | SseJsonEvent::Error => return events,
					SseJsonEvent::Done => {
						if !flushed {
							flushed = true;
							flush_end(&mut events, &mut st, &log);
						}
						return events;
					},
					SseJsonEvent::Data(Err(e)) => {
						tracing::warn!(
							"Failed to parse OpenAI-compatible stream response during translation: {}",
							e
						);
						return events;
					},
					SseJsonEvent::Data(Ok(chunk)) => {
						if !sent_created {
							sent_created = true;
							st.model = chunk.model.clone();

							let response_builder =
								types::responses::ResponseBuilder::new(st.response_id.clone(), chunk.model.clone());

							let seq = st.next_seq();
							events.push(("event", response_builder.created_event(seq)));

							// The API emits `response.in_progress` right after `response.created`.
							let seq = st.next_seq();
							events.push((
								"event",
								ResponseStreamEvent::ResponseInProgress(ResponseInProgressEvent {
									sequence_number: seq,
									response: response_builder.response(
										responses::Status::InProgress,
										None,
										None,
										None,
									),
								}),
							));

							log.update(|r| {
								r.response.provider_model = Some(strng::new(&chunk.model));
								if let Some(st) = &chunk.service_tier {
									r.response.service_tier = Some(strng::new(st));
								}
							});
						}

						if let Some(usage) = chunk.usage {
							st.pending_usage = Some(usage);
						}

						if let Some(choice) = chunk.choices.first() {
							// Reasoning streams before the answer; give it the first output index.
							if let Some(reasoning) = choice
								.delta
								.reasoning_content
								.as_deref()
								.filter(|r| !r.is_empty())
							{
								if st.reasoning_index.is_none() {
									let idx = st.take_output_index();
									st.reasoning_index = Some(idx);

									let seq = st.next_seq();
									events.push((
										"event",
										ResponseStreamEvent::ResponseOutputItemAdded(ResponseOutputItemAddedEvent {
											sequence_number: seq,
											output_index: idx,
											item: OutputItem::Reasoning(ReasoningItem {
												id: Some(st.reasoning_item_id.clone()),
												summary: Vec::new(),
												content: None,
												encrypted_content: None,
												status: Some(OutputStatus::InProgress),
											}),
										}),
									));

									let seq = st.next_seq();
									events.push((
										"event",
										ResponseStreamEvent::ResponseReasoningSummaryPartAdded(
											ResponseReasoningSummaryPartAddedEvent {
												sequence_number: seq,
												item_id: st.reasoning_item_id.clone(),
												output_index: idx,
												summary_index: 0,
												part: SummaryPart::SummaryText(SummaryTextContent {
													text: String::new(),
												}),
											},
										),
									));
								}

								if !saw_token {
									saw_token = true;
									log.update(|r| {
										r.response.first_token = Some(Instant::now());
									});
								}

								st.reasoning.push_str(reasoning);
								let seq = st.next_seq();
								events.push((
									"event",
									ResponseStreamEvent::ResponseReasoningSummaryTextDelta(
										ResponseReasoningSummaryTextDeltaEvent {
											sequence_number: seq,
											item_id: st.reasoning_item_id.clone(),
											output_index: st.reasoning_index.unwrap_or(0),
											summary_index: 0,
											delta: reasoning.to_string(),
										},
									),
								));
							}

							if let Some(content) = &choice.delta.content {
								if let Some(completion) = st.completion.as_mut() {
									completion.push_str(content);
								}

								let message_index = match st.message_index {
									Some(idx) => idx,
									None => {
										let idx = st.take_output_index();
										st.message_index = Some(idx);
										let seq = st.next_seq();
										events.push((
											"event",
											ResponseStreamEvent::ResponseOutputItemAdded(ResponseOutputItemAddedEvent {
												sequence_number: seq,
												output_index: idx,
												item: OutputItem::Message(OutputMessage {
													content: Vec::new(),
													id: st.message_item_id.clone(),
													role: AssistantRole::Assistant,
													phase: None,
													status: OutputStatus::InProgress,
												}),
											}),
										));
										idx
									},
								};

								if !st.sent_content_part {
									st.sent_content_part = true;
									let seq = st.next_seq();
									events.push((
										"event",
										ResponseStreamEvent::ResponseContentPartAdded(ResponseContentPartAddedEvent {
											sequence_number: seq,
											item_id: st.message_item_id.clone(),
											output_index: message_index,
											content_index: 0,
											part: OutputContent::OutputText(OutputTextContent {
												text: String::new(),
												annotations: Vec::new(),
												logprobs: None,
											}),
										}),
									));
								}

								if !saw_token {
									saw_token = true;
									log.update(|r| {
										r.response.first_token = Some(Instant::now());
									});
								}

								st.text.push_str(content);
								let seq = st.next_seq();
								events.push((
									"event",
									ResponseStreamEvent::ResponseOutputTextDelta(ResponseTextDeltaEvent {
										sequence_number: seq,
										item_id: st.message_item_id.clone(),
										output_index: message_index,
										content_index: 0,
										delta: content.clone(),
										logprobs: None,
									}),
								));
							}

							if let Some(tcs) = &choice.delta.tool_calls {
								for tc in tcs {
									let tool_index = tc.index;
									if let Some(logged_tool_calls) = st.logged_tool_calls.as_mut() {
										let logged_entry = logged_tool_calls.entry(tool_index).or_default();
										if let Some(id) = &tc.id {
											logged_entry.0 = Some(id.clone());
										}
										if let Some(function) = &tc.function {
											if let Some(name) = &function.name {
												logged_entry.1 = Some(name.clone());
											}
											if let Some(args) = &function.arguments {
												logged_entry.2.push_str(args);
											}
										}
									}

									let is_new = !st.tool_calls.contains_key(&tool_index);
									if is_new {
										let item_id = format!("call_{:016x}", rand::rng().random::<u64>());
										let output_index = st.take_output_index();
										st.tool_calls.insert(
											tool_index,
											(item_id, String::new(), String::new(), output_index),
										);
									}
									let entry = st
										.tool_calls
										.get_mut(&tool_index)
										.expect("tool call entry was just inserted");

									if let Some(function) = &tc.function {
										if let Some(name) = &function.name {
											entry.1 = name.clone();
										}
										if let Some(args) = &function.arguments {
											entry.2.push_str(args);
										}
									}
									let (item_id, name, _, output_index) =
										(entry.0.clone(), entry.1.clone(), (), entry.3);

									if is_new {
										if !saw_token {
											saw_token = true;
											log.update(|r| {
												r.response.first_token = Some(Instant::now());
											});
										}

										let seq = st.next_seq();
										events.push((
											"event",
											ResponseStreamEvent::ResponseOutputItemAdded(ResponseOutputItemAddedEvent {
												sequence_number: seq,
												output_index,
												item: OutputItem::FunctionCall(FunctionToolCall {
													arguments: String::new(),
													call_id: item_id.clone(),
													namespace: None,
													name: name.clone(),
													caller: None,
													id: Some(item_id.clone()),
													status: Some(OutputStatus::InProgress),
												}),
											}),
										));
									}

									if let Some(function) = &tc.function
										&& let Some(args) = &function.arguments
										&& !args.is_empty()
									{
										let seq = st.next_seq();
										events.push((
											"event",
											ResponseStreamEvent::ResponseFunctionCallArgumentsDelta(
												ResponseFunctionCallArgumentsDeltaEvent {
													sequence_number: seq,
													item_id,
													output_index,
													delta: args.clone(),
												},
											),
										));
									}
								}
							}

							if let Some(reason) = &choice.finish_reason {
								st.pending_stop_reason = Some(*reason);
							}
						}

						if !flushed && st.pending_stop_reason.is_some() && st.pending_usage.is_some() {
							flushed = true;
							flush_end(&mut events, &mut st, &log);
						}
					},
				}

				events
			},
		)
	}

	fn usage_output_tokens(usage: &completions::Usage) -> u32 {
		if usage.completion_tokens == 0 && usage.total_tokens > 0 {
			return usage.total_tokens.saturating_sub(usage.prompt_tokens);
		}
		usage.completion_tokens
	}

	fn flush_end(
		events: &mut Vec<(&'static str, responses::ResponseStreamEvent)>,
		st: &mut StreamState,
		log: &StreamingUsageGuard,
	) {
		use responses::{
			AssistantRole, ErrorObject, FunctionToolCall, IncompleteDetails, InputTokenDetails,
			OutputContent, OutputItem, OutputMessage, OutputMessageContent, OutputStatus,
			OutputTextContent, OutputTokenDetails, ReasoningItem, ResponseCompletedEvent,
			ResponseContentPartDoneEvent, ResponseFailedEvent, ResponseFunctionCallArgumentsDoneEvent,
			ResponseIncompleteEvent, ResponseOutputItemDoneEvent, ResponseReasoningSummaryPartDoneEvent,
			ResponseReasoningSummaryTextDoneEvent, ResponseStreamEvent, ResponseTextDoneEvent,
			ResponseUsage, SummaryPart, SummaryTextContent,
		};

		let stop_reason = st.pending_stop_reason.take();
		let usage = st.pending_usage.take();
		let response_status = match stop_reason.as_ref() {
			Some(completions::FinishReason::Stop)
			| Some(completions::FinishReason::ToolCalls)
			| Some(completions::FinishReason::FunctionCall)
			| None => responses::Status::Completed,
			Some(completions::FinishReason::Length) => responses::Status::Incomplete,
			Some(completions::FinishReason::ContentFilter) => responses::Status::Failed,
		};
		let finish_reason = crate::types::serialize_str(&response_status);
		let tool_parts = st.logged_tool_calls.as_mut().and_then(|logged_tool_calls| {
			crate::conversion::completions::finalize_streaming_tool_calls(
				logged_tool_calls
					.drain()
					.map(|(idx, (id, name, arguments))| (idx, id, name, arguments)),
			)
		});
		let mut tool_parts = tool_parts;
		let mut finish_reason = finish_reason;
		log.update(|r| {
			if let Some(completion) = st.completion.take() {
				r.response.completion = Some(vec![completion]);
			}
			crate::conversion::completions::build_output_messages(
				&mut r.response,
				tool_parts.take(),
				finish_reason.take(),
			);
		});

		// Finished items, by output index — these are what `response.completed`
		// carries and what each `output_item.done` repeats.
		let mut outputs: Vec<(u32, OutputItem)> = Vec::new();

		if let Some(idx) = st.reasoning_index {
			let text = std::mem::take(&mut st.reasoning);
			let part = SummaryPart::SummaryText(SummaryTextContent { text: text.clone() });

			let seq = st.next_seq();
			events.push((
				"event",
				ResponseStreamEvent::ResponseReasoningSummaryTextDone(
					ResponseReasoningSummaryTextDoneEvent {
						sequence_number: seq,
						item_id: st.reasoning_item_id.clone(),
						output_index: idx,
						summary_index: 0,
						text,
					},
				),
			));
			let seq = st.next_seq();
			events.push((
				"event",
				ResponseStreamEvent::ResponseReasoningSummaryPartDone(
					ResponseReasoningSummaryPartDoneEvent {
						sequence_number: seq,
						item_id: st.reasoning_item_id.clone(),
						output_index: idx,
						summary_index: 0,
						part: part.clone(),
						status: None,
					},
				),
			));
			let item = OutputItem::Reasoning(ReasoningItem {
				id: Some(st.reasoning_item_id.clone()),
				summary: vec![part],
				content: None,
				encrypted_content: None,
				status: Some(OutputStatus::Completed),
			});
			let seq = st.next_seq();
			events.push((
				"event",
				ResponseStreamEvent::ResponseOutputItemDone(ResponseOutputItemDoneEvent {
					sequence_number: seq,
					output_index: idx,
					item: item.clone(),
				}),
			));
			outputs.push((idx, item));
		}

		let mut sorted_tools: Vec<_> = st.tool_calls.drain().collect();
		sorted_tools.sort_by_key(|(_, (_, _, _, output_index))| *output_index);

		for (_, (item_id, name, buffer, output_index)) in sorted_tools {
			let seq = st.next_seq();
			events.push((
				"event",
				ResponseStreamEvent::ResponseFunctionCallArgumentsDone(
					ResponseFunctionCallArgumentsDoneEvent {
						sequence_number: seq,
						output_index,
						name: Some(name.clone()),
						item_id: item_id.clone(),
						arguments: buffer.clone(),
					},
				),
			));

			let item = OutputItem::FunctionCall(FunctionToolCall {
				arguments: buffer,
				call_id: item_id.clone(),
				namespace: None,
				name,
				caller: None,
				id: Some(item_id),
				status: Some(OutputStatus::Completed),
			});
			let seq = st.next_seq();
			events.push((
				"event",
				ResponseStreamEvent::ResponseOutputItemDone(ResponseOutputItemDoneEvent {
					sequence_number: seq,
					output_index,
					item: item.clone(),
				}),
			));
			outputs.push((output_index, item));
		}

		if let Some(idx) = st.message_index {
			let text = std::mem::take(&mut st.text);
			let mut content = Vec::new();
			if st.sent_content_part {
				let seq = st.next_seq();
				events.push((
					"event",
					ResponseStreamEvent::ResponseOutputTextDone(ResponseTextDoneEvent {
						sequence_number: seq,
						item_id: st.message_item_id.clone(),
						output_index: idx,
						content_index: 0,
						text: text.clone(),
						logprobs: None,
					}),
				));
				let seq = st.next_seq();
				events.push((
					"event",
					ResponseStreamEvent::ResponseContentPartDone(ResponseContentPartDoneEvent {
						sequence_number: seq,
						item_id: st.message_item_id.clone(),
						output_index: idx,
						content_index: 0,
						part: OutputContent::OutputText(OutputTextContent {
							annotations: Vec::new(),
							logprobs: None,
							text: text.clone(),
						}),
					}),
				));
				content.push(OutputMessageContent::OutputText(OutputTextContent {
					annotations: Vec::new(),
					logprobs: None,
					text,
				}));
			}
			let item = OutputItem::Message(OutputMessage {
				content,
				id: st.message_item_id.clone(),
				role: AssistantRole::Assistant,
				phase: None,
				status: OutputStatus::Completed,
			});
			let seq = st.next_seq();
			events.push((
				"event",
				ResponseStreamEvent::ResponseOutputItemDone(ResponseOutputItemDoneEvent {
					sequence_number: seq,
					output_index: idx,
					item: item.clone(),
				}),
			));
			outputs.push((idx, item));
		}

		if let Some(ref u) = usage {
			log.update(|r| {
				r.response.input_tokens = Some(u.prompt_tokens as u64);
				r.response.output_tokens = Some(usage_output_tokens(u) as u64);
				r.response.total_tokens = Some(u.total_tokens as u64);
				r.response.cached_input_tokens = u
					.prompt_tokens_details
					.as_ref()
					.and_then(|d| d.cached_tokens);
				r.response.cache_creation_input_tokens = u
					.prompt_tokens_details
					.as_ref()
					.and_then(|d| d.cache_write_tokens)
					.or(u.cache_creation_input_tokens);
				r.response.reasoning_tokens = u
					.completion_tokens_details
					.as_ref()
					.and_then(|d| d.reasoning_tokens);
			});
		}

		let usage_obj = usage.map(|u| ResponseUsage {
			input_tokens: u.prompt_tokens,
			output_tokens: usage_output_tokens(&u),
			total_tokens: u.total_tokens,
			input_tokens_details: InputTokenDetails {
				cached_tokens: u
					.prompt_tokens_details
					.as_ref()
					.and_then(|d| d.cached_tokens)
					.unwrap_or(0) as u32,
				cache_write_tokens: u
					.prompt_tokens_details
					.as_ref()
					.and_then(|d| d.cache_write_tokens)
					.or(u.cache_creation_input_tokens)
					.map(|tokens| tokens as u32),
			},
			output_tokens_details: OutputTokenDetails {
				reasoning_tokens: u
					.completion_tokens_details
					.as_ref()
					.and_then(|d| d.reasoning_tokens)
					.unwrap_or(0) as u32,
			},
		});

		outputs.sort_by_key(|(idx, _)| *idx);
		let output: Vec<OutputItem> = outputs.into_iter().map(|(_, item)| item).collect();

		let response_builder =
			types::responses::ResponseBuilder::new(st.response_id.clone(), st.model.clone());
		let seq = st.next_seq();
		let done_event = match stop_reason {
			Some(completions::FinishReason::Length) => {
				let mut response = response_builder.response(
					responses::Status::Incomplete,
					usage_obj,
					None,
					Some(IncompleteDetails {
						reason: "max_tokens".to_string(),
					}),
				);
				response.output = output;
				ResponseStreamEvent::ResponseIncomplete(ResponseIncompleteEvent {
					sequence_number: seq,
					response,
				})
			},
			Some(completions::FinishReason::ContentFilter) => {
				let mut response = response_builder.response(
					responses::Status::Failed,
					usage_obj,
					Some(ErrorObject {
						code: "content_filter".to_string(),
						message: "Content filtered".to_string(),
					}),
					None,
				);
				response.output = output;
				ResponseStreamEvent::ResponseFailed(ResponseFailedEvent {
					sequence_number: seq,
					response,
				})
			},
			Some(completions::FinishReason::Stop)
			| Some(completions::FinishReason::ToolCalls)
			| Some(completions::FinishReason::FunctionCall)
			| None => {
				let mut response =
					response_builder.response(responses::Status::Completed, usage_obj, None, None);
				response.output = output;
				ResponseStreamEvent::ResponseCompleted(ResponseCompletedEvent {
					sequence_number: seq,
					response,
				})
			},
		};

		events.push(("event", done_event));
	}
}

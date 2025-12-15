use ::http::Uri;
use ::http::header::CONTENT_TYPE;
use anyhow::anyhow;
use futures::StreamExt;
use opentelemetry::trace::Span;
use opentelemetry::trace::{SpanContext, SpanKind, TraceContextExt, Tracer as _};
use opentelemetry::{Context, KeyValue};
use reqwest::header::ACCEPT;
use rmcp::model::ConstString;
use rmcp::model::{
	ClientJsonRpcMessage, ClientNotification, ClientRequest, JsonRpcRequest, ServerJsonRpcMessage,
};
use rmcp::transport::common::http_header::{
	EVENT_STREAM_MIME_TYPE, HEADER_SESSION_ID, JSON_MIME_TYPE,
};
use rmcp::transport::streamable_http_client::StreamableHttpPostResponse;
use sse_stream::SseStream;
use std::time::Instant;

use crate::client::ResolvedDestination;
use crate::http::Request;
use crate::mcp::ClientError;
use crate::mcp::upstream::IncomingRequestContext;
use crate::telemetry::metrics::{MCPClientOpLabels, MCPClientSessionLabels};
use crate::{json, *};

#[derive(Clone, Debug)]
pub struct Client {
	http_client: super::McpHttpClient,
	uri: Uri,
	session_id: AtomicOption<String>,
	// When set (e.g., after establishing SSE), this span context is used
	// as the explicit parent for future upstream spans.
	otel_parent: AtomicOption<SpanContext>,
	// Persist the negotiated MCP protocol version (e.g., from initialize).
	protocol_version: AtomicOption<String>,
	// Metrics handle
	metrics: Arc<crate::telemetry::metrics::Metrics>,
	// Session start time (first time we observe a session id)
	session_start: AtomicOption<Instant>,
}

impl Client {
	pub fn new(http_client: super::McpHttpClient, path: Strng) -> anyhow::Result<Self> {
		let hp = http_client.backend().hostport();
		let metrics = http_client.metrics().clone();
		Ok(Self {
			http_client,
			uri: ("http://".to_string() + &hp + path.as_str()).parse()?,
			session_id: Default::default(),
			otel_parent: Default::default(),
			protocol_version: Default::default(),
			metrics,
			session_start: Default::default(),
		})
	}
	pub fn set_session_id(&self, s: String) {
		self.session_id.store(Some(Arc::new(s)));
	}

	pub async fn send_request(
		&self,
		req: JsonRpcRequest<ClientRequest>,

		ctx: &IncomingRequestContext,
	) -> Result<StreamableHttpPostResponse, ClientError> {
		let message = ClientJsonRpcMessage::Request(req);
		self.send_message(message, ctx).await
	}
	pub async fn send_notification(
		&self,
		req: ClientNotification,

		ctx: &IncomingRequestContext,
	) -> Result<StreamableHttpPostResponse, ClientError> {
		let message = ClientJsonRpcMessage::notification(req);
		self.send_message(message, ctx).await
	}
	async fn send_message(
		&self,
		message: ClientJsonRpcMessage,

		ctx: &IncomingRequestContext,
	) -> Result<StreamableHttpPostResponse, ClientError> {
		let op_start = Instant::now();
		let body = serde_json::to_vec(&message).map_err(ClientError::new)?;
		let tracer = agent_core::trcng::get_tracer();
		let parent_cx: Context = match &*self.otel_parent.load() {
			Some(sc) => {
				// Use previously stored parent (e.g., from initial call) to ensure
				// downstream spans continue the same trace lineage.
				Context::new().with_remote_span_context(sc.as_ref().clone())
			},
			None => ctx.otel_parent_context(),
		};
		// Parse the outgoing JSON-RPC message for attribute extraction where applicable.
		let parsed_body: Option<serde_json::Value> = serde_json::from_slice(&body).ok();
		let params_obj = parsed_body.as_ref().and_then(|v| v.get("params"));
		let resource_uri_attr = params_obj
			.and_then(|p| p.get("uri"))
			.and_then(|u| u.as_str())
			.map(|s| s.to_string());
		// Try to extract the MCP protocol version from params (supports both snake_case and camelCase).
		let protocol_version_in_params = params_obj
			.and_then(|p| {
				p.get("protocol_version")
					.or_else(|| p.get("protocolVersion"))
			})
			.and_then(|v| v.as_str())
			.map(|s| s.to_string());
		let method_in_body = parsed_body
			.as_ref()
			.and_then(|v| v.get("method"))
			.and_then(|m| m.as_str())
			.map(|s| s.to_string());
		let (method_name, request_id) = match &message {
			ClientJsonRpcMessage::Request(r) => (
				Some(r.request.method().to_string()),
				Some(format!("{}", r.id)),
			),
			ClientJsonRpcMessage::Notification(n) => {
				let method = match &n.notification {
					ClientNotification::CancelledNotification(r) => r.method.as_str(),
					ClientNotification::ProgressNotification(r) => r.method.as_str(),
					ClientNotification::InitializedNotification(r) => r.method.as_str(),
					ClientNotification::RootsListChangedNotification(r) => r.method.as_str(),
				};
				(Some(method.to_string()), None)
			},
			ClientJsonRpcMessage::Response(_) => (None, None),
			ClientJsonRpcMessage::Error(_) => (None, None),
		};
		let (server_addr, server_port) = match self.uri.authority() {
			Some(a) => {
				let host = a.host().to_string();
				let port = a.port_u16().unwrap_or(80);
				(host, port)
			},
			None => ("unknown".to_string(), 0),
		};
		let mut attrs = vec![
			KeyValue::new("rpc.system", "jsonrpc"),
			KeyValue::new("rpc.jsonrpc.version", "2.0"),
			KeyValue::new("server.address", server_addr.clone()),
			KeyValue::new("server.port", server_port as i64),
			KeyValue::new("network.protocol.name", "http"),
			KeyValue::new("rpc.request.size", body.len() as i64),
			KeyValue::new(
				"mcp.session.id",
				self
					.session_id
					.load()
					.as_deref()
					.map_or("unknown", String::as_str)
					.to_string(),
			),
		];
		if let Some(m) = &method_name {
			attrs.push(KeyValue::new("rpc.method", m.clone()));
		}
		// Registry attribute: mcp.method.name
		if let Some(m) = method_name.clone().or(method_in_body.clone()) {
			attrs.push(KeyValue::new("mcp.method.name", m));
		}
		if let Some(id) = &request_id {
			// Use MCP-consistent attribute name for request id.
			attrs.push(KeyValue::new("mcp.request.id", id.clone()));
		}
		// If a resource URI parameter is present, record it.
		if let Some(uri) = &resource_uri_attr {
			attrs.push(KeyValue::new("mcp.resource.uri", uri.clone()));
		}
		// Registry attribute: mcp.protocol.version (prefer params; else use stored)
		if let Some(pv) = &protocol_version_in_params {
			self.protocol_version.store(Some(Arc::new(pv.clone())));
			attrs.push(KeyValue::new("mcp.protocol.version", pv.clone()));
		} else if let Some(pv) = self.protocol_version.load().as_deref() {
			attrs.push(KeyValue::new("mcp.protocol.version", pv.to_string()));
		}
		// If this is a tools/call operation, record the call arguments when available.
		let effective_method = method_name.as_ref().or(method_in_body.as_ref());
		if matches!(effective_method.map(|s| s.as_str()), Some("tools/call"))
			&& let Some(arguments_value) = params_obj.and_then(|p| p.get("arguments"))
			&& let Ok(mut arguments_str) = serde_json::to_string(arguments_value)
		{
			const MAX_ATTR_LEN: usize = 2048;
			if arguments_str.len() > MAX_ATTR_LEN {
				arguments_str.truncate(MAX_ATTR_LEN);
			}
			attrs.push(KeyValue::new("gen_ai.tool.call.arguments", arguments_str));
		}
		attrs.push(KeyValue::new(
			"rpc.service",
			self
				.uri
				.authority()
				.map(|a| a.as_str().to_string())
				.unwrap_or_else(|| "unknown".to_string()),
		));
		let mut span = tracer
			.span_builder(
				method_name
					.as_deref()
					.map(|m| format!("mcp.upstream.{m}"))
					.unwrap_or_else(|| "mcp.upstream.call".to_string()),
			)
			.with_kind(SpanKind::Client)
			.with_attributes(attrs)
			.start_with_context(tracer, &parent_cx);

		// Create a derived context containing this span's context so we can
		// explicitly parent subsequent spans, even after this span ends.
		let derived_parent_for_children: SpanContext = span.span_context().clone();

		let mut req = ::http::Request::builder()
			.uri(&self.uri)
			.method(http::Method::POST)
			.header(CONTENT_TYPE, "application/json")
			.header(ACCEPT, [EVENT_STREAM_MIME_TYPE, JSON_MIME_TYPE].join(", "))
			.body(body.into())
			.map_err(ClientError::new)?;

		self.maybe_insert_session_id(&mut req)?;

		ctx.apply(&mut req);

		let resp = self.http_client.call(req).await.map_err(ClientError::new)?;

		if resp.status() == http::StatusCode::ACCEPTED {
			span.end();
			return Ok(StreamableHttpPostResponse::Accepted);
		}

		if !resp.status().is_success() {
			span.set_attribute(KeyValue::new(
				"rpc.error.code",
				resp.status().as_u16() as i64,
			));
			span.set_attribute(KeyValue::new(
				"rpc.error.message",
				format!("http {}", resp.status()),
			));
			span.end();
			return Err(ClientError::Status(Box::new(resp)));
		}

		let content_type = resp.headers().get(CONTENT_TYPE);
		let session_id = resp
			.headers()
			.get(HEADER_SESSION_ID)
			.and_then(|v| v.to_str().ok())
			.map(|s| s.to_string());
		if let Some(sid) = &session_id {
			span.set_attribute(KeyValue::new("mcp.session.id", sid.clone()));
			// Initialize session start if not set
			if self.session_start.load().is_none() {
				self.session_start.store(Some(Arc::new(op_start)));
			}
		}
		if let Some(resolved) = resp.extensions().get::<ResolvedDestination>() {
			span.set_attribute(KeyValue::new(
				"mcp.session.pinned_endpoint",
				resolved.0.to_string(),
			));
		}
		if let Some(len) = resp
			.headers()
			.get(::http::header::CONTENT_LENGTH)
			.and_then(|v| v.to_str().ok())
			.and_then(|s| s.parse::<i64>().ok())
		{
			span.set_attribute(KeyValue::new("rpc.response.size", len));
		}

		// Record client operation duration metric
		{
			let labels = MCPClientOpLabels {
				server_address: server_addr.clone().into(),
				server_port: Some(server_port).into(),
			};
			self
				.metrics
				.mcp_client_operation_duration
				.get_or_create(&labels)
				.observe(op_start.elapsed().as_secs_f64());
		}

		match content_type {
			Some(ct) if ct.as_bytes().starts_with(EVENT_STREAM_MIME_TYPE.as_bytes()) => {
				let event_stream = SseStream::from_byte_stream(resp.into_body().into_data_stream()).boxed();
				span.set_attribute(KeyValue::new("rpc.response.mode", "sse"));
				// Persist this span context to be used as the explicit parent for future spans.
				self
					.otel_parent
					.store(Some(Arc::new(derived_parent_for_children)));
				span.end();
				Ok(StreamableHttpPostResponse::Sse(event_stream, session_id))
			},
			Some(ct) if ct.as_bytes().starts_with(JSON_MIME_TYPE.as_bytes()) => {
				let message = json::from_response_body::<ServerJsonRpcMessage>(resp)
					.await
					.map_err(ClientError::new)?;
				if let ServerJsonRpcMessage::Response(r) = &message {
					use rmcp::model::ServerResult::*;
					match &r.result {
						ListToolsResult(v) => span.set_attribute(KeyValue::new(
							"mcp.response.tools_count",
							v.tools.len() as i64,
						)),
						ListResourcesResult(v) => span.set_attribute(KeyValue::new(
							"mcp.response.resources_count",
							v.resources.len() as i64,
						)),
						ListPromptsResult(v) => span.set_attribute(KeyValue::new(
							"mcp.response.prompts_count",
							v.prompts.len() as i64,
						)),
						ListResourceTemplatesResult(v) => span.set_attribute(KeyValue::new(
							"mcp.response.resource_templates_count",
							v.resource_templates.len() as i64,
						)),
						_ => {},
					}
				}
				// If this was a tools/call, attempt to record the call result (truncated).
				if matches!(method_name.as_deref(), Some("tools/call"))
					&& let Ok(mut val) = serde_json::to_value(&message)
				{
					let result_value = val
						.get_mut("result")
						.cloned()
						.unwrap_or(serde_json::Value::Null);
					if let Ok(mut result_str) = serde_json::to_string(&result_value) {
						const MAX_ATTR_LEN: usize = 2048;
						if result_str.len() > MAX_ATTR_LEN {
							result_str.truncate(MAX_ATTR_LEN);
						}
						span.set_attribute(KeyValue::new("gen_ai.tool.call.result", result_str));
					}
				}
				span.set_attribute(KeyValue::new("rpc.response.mode", "json"));
				span.end();
				Ok(StreamableHttpPostResponse::Json(message, session_id))
			},
			_ => Err(ClientError::new(anyhow!(
				"unexpected content type: {:?}",
				content_type
			))),
		}
	}
	pub async fn send_delete(
		&self,

		ctx: &IncomingRequestContext,
	) -> Result<StreamableHttpPostResponse, ClientError> {
		let session_end = Instant::now();
		let mut req = ::http::Request::builder()
			.uri(&self.uri)
			.method(http::Method::DELETE)
			.body(crate::http::Body::empty())
			.map_err(ClientError::new)?;

		self.maybe_insert_session_id(&mut req)?;

		ctx.apply(&mut req);

		let resp = self.http_client.call(req).await.map_err(ClientError::new)?;

		if !resp.status().is_success() {
			return Err(ClientError::Status(Box::new(resp)));
		}
		// If we have a session, record its duration now that we've ended it.
		if let Some(start) = self.session_start.load().as_deref() {
			let (server_addr, server_port) = match self.uri.authority() {
				Some(a) => {
					let host = a.host().to_string();
					let port = a.port_u16().unwrap_or(80);
					(host, port)
				},
				None => ("unknown".to_string(), 0),
			};
			let labels = MCPClientSessionLabels {
				server_address: server_addr.into(),
				server_port: Some(server_port).into(),
			};
			self
				.metrics
				.mcp_client_session_duration
				.get_or_create(&labels)
				.observe(session_end.saturating_duration_since(*start).as_secs_f64());
			// Clear session tracking
			self.session_start.store(None);
			self.session_id.store(None);
		}
		Ok(StreamableHttpPostResponse::Accepted)
	}
	pub async fn get_event_stream(
		&self,
		ctx: &IncomingRequestContext,
	) -> Result<StreamableHttpPostResponse, ClientError> {
		let mut req = ::http::Request::builder()
			.uri(&self.uri)
			.method(http::Method::GET)
			.header(ACCEPT, EVENT_STREAM_MIME_TYPE)
			.body(crate::http::Body::empty())
			.map_err(ClientError::new)?;

		self.maybe_insert_session_id(&mut req)?;

		ctx.apply(&mut req);

		let resp = self.http_client.call(req).await.map_err(ClientError::new)?;

		if !resp.status().is_success() {
			return Err(ClientError::Status(Box::new(resp)));
		}

		let content_type = resp.headers().get(CONTENT_TYPE);
		let session_id = resp
			.headers()
			.get(HEADER_SESSION_ID)
			.and_then(|v| v.to_str().ok())
			.map(|s| s.to_string());
		match content_type {
			Some(ct) if ct.as_bytes().starts_with(EVENT_STREAM_MIME_TYPE.as_bytes()) => {
				let event_stream = SseStream::from_byte_stream(resp.into_body().into_data_stream()).boxed();
				Ok(StreamableHttpPostResponse::Sse(event_stream, session_id))
			},
			_ => Err(ClientError::new(anyhow!(
				"unexpected content type for GET streams: {:?}",
				content_type
			))),
		}
	}

	fn maybe_insert_session_id(&self, req: &mut Request) -> Result<(), ClientError> {
		if let Some(session_id) = self.session_id.load().clone() {
			req.headers_mut().insert(
				HEADER_SESSION_ID,
				session_id.as_ref().parse().map_err(ClientError::new)?,
			);
		}
		Ok(())
	}
}

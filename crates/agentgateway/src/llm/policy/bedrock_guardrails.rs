use agent_core::strng;
use itertools::Itertools;
use serde::{Deserialize, Serialize};

use crate::http::jwt::Claims;
use crate::json;
use crate::llm::RequestType;
use crate::llm::policy::BedrockGuardrails;
use crate::proxy::httpproxy::PolicyClient;
use crate::types::agent::{BackendPolicy, ResourceName, SimpleBackend, Target};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum GuardrailSource {
	/// Content from user input (requests)
	Input,
	/// Content from model output (responses)
	Output,
}

/// Text content block for guardrail evaluation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GuardrailTextBlock {
	pub text: String,
}

/// Content block for guardrail evaluation
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct GuardrailContentBlock {
	pub text: GuardrailTextBlock,
}

/// Request body for ApplyGuardrail API
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct ApplyGuardrailRequest {
	/// The source of the content (INPUT for requests, OUTPUT for responses)
	pub source: GuardrailSource,
	/// The content blocks to evaluate
	pub content: Vec<GuardrailContentBlock>,
}

/// Action taken by the guardrail
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GuardrailAction {
	/// No intervention needed
	None,
	/// Guardrail intervened and blocked/modified content
	GuardrailIntervened,
}

/// Response from ApplyGuardrail API
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ApplyGuardrailResponse {
	/// The action taken by the guardrail
	pub action: GuardrailAction,
}

impl ApplyGuardrailResponse {
	/// Returns true if the guardrail intervened
	pub fn is_blocked(&self) -> bool {
		self.action == GuardrailAction::GuardrailIntervened
	}
}

/// Send a request to the Bedrock Guardrails ApplyGuardrail API for request content
pub async fn send_request(
	req: &mut dyn RequestType,
	claims: Option<Claims>,
	client: &PolicyClient,
	guardrails: &BedrockGuardrails,
) -> anyhow::Result<ApplyGuardrailResponse> {
	let content = req
		.get_messages()
		.into_iter()
		.map(|m| GuardrailContentBlock {
			text: GuardrailTextBlock {
				text: m.content.to_string(),
			},
		})
		.collect_vec();

	send_guardrail_request(
		client,
		claims.clone(),
		guardrails,
		GuardrailSource::Input,
		content,
	)
	.await
}

/// Send a request to the Bedrock Guardrails ApplyGuardrail API for response content
pub async fn send_response(
	content: Vec<String>,
	claims: Option<Claims>,
	client: &PolicyClient,
	guardrails: &BedrockGuardrails,
) -> anyhow::Result<ApplyGuardrailResponse> {
	let content = content
		.into_iter()
		.map(|text| GuardrailContentBlock {
			text: GuardrailTextBlock { text },
		})
		.collect_vec();

	send_guardrail_request(
		client,
		claims.clone(),
		guardrails,
		GuardrailSource::Output,
		content,
	)
	.await
}

async fn send_guardrail_request(
	client: &PolicyClient,
	claims: Option<Claims>,
	guardrails: &BedrockGuardrails,
	source: GuardrailSource,
	content: Vec<GuardrailContentBlock>,
) -> anyhow::Result<ApplyGuardrailResponse> {
	let request_body = ApplyGuardrailRequest { source, content };
	let host = strng::format!("bedrock-runtime.{}.amazonaws.com", guardrails.region);
	let path = format!(
		"/guardrail/{}/version/{}/apply",
		guardrails.guardrail_identifier, guardrails.guardrail_version
	);
	let uri = format!("https://{}{}", host, path);

	let mut pols = vec![BackendPolicy::BackendTLS(
		crate::http::backendtls::SYSTEM_TRUST.clone(),
	)];
	pols.extend(guardrails.policies.iter().cloned());

	let mut rb = ::http::Request::builder()
		.uri(&uri)
		.method(::http::Method::POST)
		.header(::http::header::CONTENT_TYPE, "application/json");

	if let Some(claims) = claims {
		rb = rb.extension(claims);
	}

	let req = rb.body(crate::http::Body::from(serde_json::to_vec(&request_body)?))?;

	let mock_be = SimpleBackend::Opaque(
		ResourceName::new(strng::literal!("_bedrock-guardrails"), strng::literal!("")),
		Target::Hostname(host, 443),
	);

	let resp = client
		.call_with_explicit_policies(req, mock_be, pols)
		.await?;

	let resp: ApplyGuardrailResponse = json::from_response_body(resp).await?;
	Ok(resp)
}

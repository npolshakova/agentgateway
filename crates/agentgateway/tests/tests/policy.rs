use agentgateway::test_helpers::{
	TEST_TRACE_ID, TEST_TRACEPARENT, TEST_TRACESTATE, ascii_metadata, extauthmock,
};

use crate::common::prelude::*;

#[tokio::test]
async fn response_policy_short_circuit() {
	let (_mock, mut bind, io) = basic_setup().await;
	bind
		.attach_route(json!({
			"policies": {
				"extAuthz": {
					// Dummy host that should fail
					"host": "127.0.0.1:1",
				},
				"responseHeaderModifier": {
					"add": {
						"x-filter": "x-filter-val"
					},
				},
				"transformations": {
					"response": {
						"add": {
							"x-xfm": "'x-xfm-val'",
						},
					},
				},
			},
		}))
		.await;

	let res = send_request(io.clone(), Method::GET, "http://lo/p").await;
	assert_eq!(res.status(), 403);
	// Each type of response modifier should NOT run since the ext_authz short-circuits the req
	assert_eq!(res.hdr("x-filter"), "");
	assert_eq!(res.hdr("x-xfm"), "");
	assert_eq!(read_body!(res).as_ref(), b"external authorization failed");
}

#[tokio::test]
async fn header_manipulation() {
	let (mock, mut bind, _io) = basic_setup().await;
	bind
		.attach_route(json!({
			"policies": {
				"requestHeaderModifier": {
					"add": {
						"x-route-req": "route-req",
					},
				},
				"responseHeaderModifier": {
					"add": {
						"x-route-resp": "route-resp",
					},
				},
			},
			"backends": [{
				"host": mock.address().to_string(),
				"policies": {
					"requestHeaderModifier": {
						"add": {
							"x-backend-req": "backend-req",
						},
					},
					"responseHeaderModifier": {
						"add": {
							"x-backend-resp": "backend-resp",
						},
					},
					"transformations": {
						"request": {
							"set": {
								"x-backend-xfm-req": "'backend-xfm-req'",
							},
						},
						"response": {
							"add": {
								"x-backend-xfm-resp": "'backend-xfm-resp'",
							},
						},
					},
				},
			}],
		}))
		.await;
	let io = bind.serve_http(BIND_KEY);

	let res = send_request(io.clone(), Method::GET, "http://lo/p").await;
	assert_eq!(res.status(), 200);
	assert_eq!(res.hdr("x-route-resp"), "route-resp");
	assert_eq!(res.hdr("x-backend-resp"), "backend-resp");
	assert_eq!(res.hdr("x-backend-xfm-resp"), "backend-xfm-resp");
	let body = read_body(res.into_body()).await;
	assert_eq!(
		body.headers.get("x-route-req").unwrap().as_bytes(),
		b"route-req"
	);
	assert_eq!(
		body.headers.get("x-backend-req").unwrap().as_bytes(),
		b"backend-req"
	);
	assert_eq!(
		body.headers.get("x-backend-xfm-req").unwrap().as_bytes(),
		b"backend-xfm-req"
	);
}

#[tokio::test]
async fn gateway_ext_authz_response_headers_are_preserved() {
	struct AddResponseHeader;

	#[async_trait::async_trait]
	impl extauthmock::Handler for AddResponseHeader {
		async fn check(
			&mut self,
			_request: &agentgateway::http::ext_authz::proto::CheckRequest,
		) -> Result<agentgateway::http::ext_authz::proto::CheckResponse, tonic::Status> {
			use agentgateway::http::ext_authz::proto::check_response::HttpResponse;
			use agentgateway::http::ext_authz::proto::{HeaderValue, HeaderValueOption, OkHttpResponse};

			extauthmock::allow_response(Some(HttpResponse::OkResponse(OkHttpResponse {
				headers: vec![],
				headers_to_remove: vec![],
				response_headers_to_add: vec![HeaderValueOption {
					header: Some(HeaderValue {
						key: "x-gateway-authz-response".to_string(),
						value: "allowed".to_string(),
						raw_value: vec![],
					}),
					append: Some(false),
					append_action: 0,
				}],
				query_parameters_to_set: vec![],
				query_parameters_to_remove: vec![],
				..Default::default()
			})))
		}
	}

	let (mock, mut bind, io) = basic_setup().await;
	let authz = extauthmock::ExtAuthMock::new(|| AddResponseHeader)
		.spawn()
		.await;
	bind
		.attach_gateway_policy(json!({
			"extAuthz": {
				"host": authz.address,
			},
		}))
		.await;

	let res = send_request(io.clone(), Method::GET, "http://lo/p").await;
	assert_eq!(res.status(), 200);
	assert_eq!(res.hdr("x-gateway-authz-response"), "allowed");
	assert_eq!(read_body(res.into_body()).await.method, Method::GET);
	drop(mock);
}

// The outbound ext_authz gRPC Check call must carry the incoming W3C trace
// context (traceparent/tracestate) as gRPC metadata so the authz service can
// join the trace.
#[tokio::test]
async fn gateway_ext_authz_grpc_check_carries_traceparent_metadata() {
	use std::collections::HashMap;

	#[derive(Clone)]
	struct MetadataRecorder {
		seen: Arc<StdMutex<Vec<HashMap<String, String>>>>,
	}

	#[async_trait::async_trait]
	impl extauthmock::Handler for MetadataRecorder {
		async fn on_check(&mut self, metadata: &tonic::metadata::MetadataMap) {
			self.seen.lock().unwrap().push(ascii_metadata(metadata));
		}
	}

	let seen = Arc::new(StdMutex::new(Vec::new()));
	let authz = extauthmock::ExtAuthMock::new({
		let seen = seen.clone();
		move || MetadataRecorder { seen: seen.clone() }
	})
	.spawn()
	.await;

	let (_mock, mut bind, io) = basic_setup().await;
	bind
		.attach_gateway_policy(json!({
			"extAuthz": {
				"host": authz.address,
			},
		}))
		.await;

	let res = send_request_headers(
		io.clone(),
		Method::GET,
		"http://lo/p",
		&[
			("traceparent", TEST_TRACEPARENT),
			("tracestate", TEST_TRACESTATE),
		],
	)
	.await;
	assert_eq!(res.status(), 200);

	let seen = seen.lock().unwrap();
	assert!(!seen.is_empty(), "authz mock should have been called");
	let tp = seen[0]
		.get("traceparent")
		.expect("ext_authz gRPC call should carry a traceparent metadata entry");
	// The gateway may start its own span (new span-id), but the trace-id must be
	// preserved so the authz service can join the trace.
	assert_eq!(
		tp.split('-').nth(1),
		Some(TEST_TRACE_ID),
		"traceparent metadata should preserve the incoming trace-id: {tp}"
	);
	assert_eq!(
		seen[0].get("tracestate").map(String::as_str),
		Some(TEST_TRACESTATE),
		"ext_authz gRPC call should carry the incoming tracestate as metadata"
	);
}

// The outbound HTTP ext_authz check request must carry the incoming W3C trace
// context (traceparent/tracestate) as HTTP headers.
#[tokio::test]
async fn gateway_http_ext_authz_check_request_carries_traceparent() {
	let (_mock, mut bind, io) = basic_setup().await;
	let authz = MockServer::start().await;
	let seen_headers: Arc<StdMutex<Vec<HeaderMap>>> = Arc::new(StdMutex::new(Vec::new()));
	let seen_clone = seen_headers.clone();
	Mock::given(wiremock::matchers::any())
		.respond_with(move |req: &wiremock::Request| {
			seen_clone.lock().unwrap().push(req.headers.clone());
			ResponseTemplate::new(StatusCode::OK.as_u16())
		})
		.mount(&authz)
		.await;

	bind
		.attach_gateway_policy(json!({
			"extAuthz": {
				"host": authz.address().to_string(),
				"protocol": {"http": {}},
			},
		}))
		.await;

	let res = send_request_headers(
		io.clone(),
		Method::GET,
		"http://lo/p",
		&[
			("traceparent", TEST_TRACEPARENT),
			("tracestate", TEST_TRACESTATE),
		],
	)
	.await;
	assert_eq!(res.status(), StatusCode::OK);

	let seen = seen_headers.lock().unwrap();
	assert!(!seen.is_empty(), "authz mock should have been called");
	let tp = seen[0]
		.get("traceparent")
		.and_then(|v| v.to_str().ok())
		.expect("HTTP ext_authz check request should carry a traceparent header");
	// The gateway may start its own span (new span-id), but the trace-id must be
	// preserved so the authz service can join the trace.
	assert_eq!(
		tp.split('-').nth(1),
		Some(TEST_TRACE_ID),
		"traceparent header should preserve the incoming trace-id: {tp}"
	);
	assert_eq!(
		seen[0].get("tracestate").and_then(|v| v.to_str().ok()),
		Some(TEST_TRACESTATE),
		"HTTP ext_authz check request should carry the incoming tracestate header"
	);
}

// A traceparent explicitly set by the user via addRequestHeaders must win over
// automatic trace context propagation, matching the gRPC precedence rule.
#[tokio::test]
async fn gateway_http_ext_authz_user_traceparent_takes_precedence() {
	const USER_TRACEPARENT: &str = "00-11111111111111111111111111111111-2222222222222222-01";

	let (_mock, mut bind, io) = basic_setup().await;
	let authz = MockServer::start().await;
	let seen_headers: Arc<StdMutex<Vec<HeaderMap>>> = Arc::new(StdMutex::new(Vec::new()));
	let seen_clone = seen_headers.clone();
	Mock::given(wiremock::matchers::any())
		.respond_with(move |req: &wiremock::Request| {
			seen_clone.lock().unwrap().push(req.headers.clone());
			ResponseTemplate::new(StatusCode::OK.as_u16())
		})
		.mount(&authz)
		.await;

	bind
		.attach_gateway_policy(json!({
			"extAuthz": {
				"host": authz.address().to_string(),
				"protocol": {"http": {
					"addRequestHeaders": {
						"traceparent": format!("'{USER_TRACEPARENT}'"),
					},
				}},
			},
		}))
		.await;

	let res = send_request_headers(
		io.clone(),
		Method::GET,
		"http://lo/p",
		&[("traceparent", TEST_TRACEPARENT)],
	)
	.await;
	assert_eq!(res.status(), StatusCode::OK);

	let seen = seen_headers.lock().unwrap();
	assert!(!seen.is_empty(), "authz mock should have been called");
	assert_eq!(
		seen[0].get("traceparent").and_then(|v| v.to_str().ok()),
		Some(USER_TRACEPARENT),
		"user-configured traceparent must not be overwritten by propagation"
	);
}

#[tokio::test]
async fn gateway_http_ext_authz_caches_unauthorized_response() {
	let (_mock, mut bind, io) = basic_setup().await;
	let authz = MockServer::start().await;
	let calls = Arc::new(AtomicUsize::new(0));
	let calls_clone = calls.clone();
	Mock::given(wiremock::matchers::any())
		.respond_with(move |_req: &wiremock::Request| {
			let call = calls_clone.fetch_add(1, Ordering::SeqCst) + 1;
			ResponseTemplate::new(StatusCode::UNAUTHORIZED.as_u16())
				.set_body_string(format!("authz-denied-{call}"))
		})
		.mount(&authz)
		.await;

	bind
		.attach_gateway_policy(json!({
			"extAuthz": {
				"host": authz.address().to_string(),
				"protocol": {"http": {}},
				"cache": {
					"key": ["request.path"],
					"ttl": "60s",
				},
			},
		}))
		.await;

	let res = send_request(io.clone(), Method::GET, "http://lo/p").await;
	assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
	assert_eq!(
		read_body_raw(res.into_body()).await.as_ref(),
		b"authz-denied-1"
	);

	let res = send_request(io.clone(), Method::GET, "http://lo/p").await;
	assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
	assert_eq!(
		read_body_raw(res.into_body()).await.as_ref(),
		b"authz-denied-1"
	);
	assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn gateway_http_ext_authz_does_not_cache_server_error_response() {
	let (_mock, mut bind, io) = basic_setup().await;
	let authz = MockServer::start().await;
	let calls = Arc::new(AtomicUsize::new(0));
	let calls_clone = calls.clone();
	Mock::given(wiremock::matchers::any())
		.respond_with(move |_req: &wiremock::Request| {
			let call = calls_clone.fetch_add(1, Ordering::SeqCst) + 1;
			ResponseTemplate::new(StatusCode::INTERNAL_SERVER_ERROR.as_u16())
				.set_body_string(format!("authz-error-{call}"))
		})
		.mount(&authz)
		.await;

	bind
		.attach_gateway_policy(json!({
			"extAuthz": {
				"host": authz.address().to_string(),
				"protocol": {"http": {}},
				"cache": {
					"key": ["request.path"],
					"ttl": "60s",
				},
			},
		}))
		.await;

	let res = send_request(io.clone(), Method::GET, "http://lo/p").await;
	assert_eq!(res.status(), StatusCode::INTERNAL_SERVER_ERROR);
	assert_eq!(
		read_body_raw(res.into_body()).await.as_ref(),
		b"authz-error-1"
	);

	let res = send_request(io.clone(), Method::GET, "http://lo/p").await;
	assert_eq!(res.status(), StatusCode::INTERNAL_SERVER_ERROR);
	assert_eq!(
		read_body_raw(res.into_body()).await.as_ref(),
		b"authz-error-2"
	);
	assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn gateway_http_ext_authz_body_expression_sets_auth_request_body() {
	let (_mock, mut bind, io) = basic_setup().await;
	let authz = MockServer::start().await;
	Mock::given(wiremock::matchers::any())
		.respond_with(move |req: &wiremock::Request| {
			ResponseTemplate::new(StatusCode::OK.as_u16()).insert_header(
				"x-authz-body",
				String::from_utf8(req.body.clone()).expect("authz request body is utf8"),
			)
		})
		.mount(&authz)
		.await;

	bind
		.attach_gateway_policy(json!({
			"extAuthz": {
				"host": authz.address().to_string(),
				"protocol": {
					"http": {
						"body": r#"{"path": request.path, "method": request.method}"#,
						"includeResponseHeaders": ["x-authz-body"],
					},
				},
			},
		}))
		.await;

	let res = send_request_body(io.clone(), Method::POST, "http://lo/p", b"original").await;
	assert_eq!(res.status(), StatusCode::OK);
	let body = read_body(res.into_body()).await;
	let authz_body: serde_json::Value =
		serde_json::from_slice(body.headers.get("x-authz-body").unwrap().as_bytes()).unwrap();
	assert_eq!(
		authz_body,
		json!({
			"path": "/p",
			"method": "POST",
		})
	);
	assert_eq!(body.body.as_ref(), b"original");
}

#[tokio::test]
async fn gateway_transformation_response_headers_are_applied() {
	let (_mock, mut bind, io) = basic_setup().await;
	bind
		.attach_gateway_policy(json!({
			"transformations": {
				"request": {
					"set": {
						"x-gateway-xfm-req": "'gateway-request'",
					},
				},
				"response": {
					"add": {
						"x-gateway-xfm-resp": "'gateway-response'",
					},
				},
			},
		}))
		.await;

	let res = send_request(io.clone(), Method::GET, "http://lo/p").await;
	assert_eq!(res.status(), 200);
	assert_eq!(res.hdr("x-gateway-xfm-resp"), "gateway-response");
	let body = read_body(res.into_body()).await;
	assert_eq!(
		body.headers.get("x-gateway-xfm-req").unwrap().as_bytes(),
		b"gateway-request"
	);
}

#[tokio::test]
async fn inline_backend_policies() {
	let (mock, mut bind, io) = basic_setup().await;
	bind
		.attach_backend(json!({
			"name": "backend1",
			"host": mock.address(),
			"policies": {
				"requestHeaderModifier": {
					"add": {
						"x-backend-req": "backend-req",
					}
				},
				"responseHeaderModifier": {
					"add": {
						"x-backend-resp": "backend-resp",
					}
				}
			}
		}))
		.await;
	bind
		.attach_route(json!({
			"policies": {
				"requestHeaderModifier": {
					"add": {
						"x-route-req": "route-req",
					},
				},
				"responseHeaderModifier": {
					"add": {
						"x-route-resp": "route-resp",
					},
				},
			},
			"backends": [{
				"backend": "/backend1",
				"policies": {
					"requestHeaderModifier": {
						"add": {
							"x-backend-route-req": "backend-route-req",
						},
					},
					"responseHeaderModifier": {
						"add": {
							"x-backend-route-resp": "backend-route-resp",
						},
					},
				},
			}],
		}))
		.await;

	let res = send_request(io.clone(), Method::GET, "http://lo/p").await;
	assert_eq!(res.status(), 200);
	// We should get the route rule, and the inline backend rule. The Backend rule takes precedence
	// over the HTTPRoute.backendRef.filters though, so that one is ignored (no deep merging, either).
	assert_eq!(res.hdr("x-route-resp"), "route-resp");
	assert_eq!(res.hdr("x-backend-route-resp"), "backend-route-resp");
	assert_eq!(res.hdr("x-backend-resp"), "");
	let body = read_body(res.into_body()).await;
	assert_eq!(
		body.headers.get("x-route-req").unwrap().as_bytes(),
		b"route-req"
	);
	assert!(body.headers.get("x-backend-req").is_none(),);
	assert_eq!(
		body.headers.get("x-backend-route-req").unwrap().as_bytes(),
		b"backend-route-req"
	);
}

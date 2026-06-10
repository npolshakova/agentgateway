# Confidential Computing vLLM Example

This example shows agentgateway routing OpenAI-compatible chat completion
requests to a vLLM model server through an InferencePool, while an agentgateway
LLM prompt-guard webhook requires a confidential-computing attestation report
before the request reaches vLLM.

The attestation verifier in this example is intentionally small and demo-only:
it checks that the client sent an expected attestation report hash in the
`x-attestation-report` header. For a real confidential-computing deployment,
replace the verifier with one that validates SGX or TDX quote evidence and
binds the report to the measured vLLM workload. You can use
https://proof.t16z.com/ to inspect an attestation quote and get a report hash
for the demo header.

The flow is:

```text
client -> Gateway -> HTTPRoute -> AgentgatewayBackend -> InferencePool -> vLLM
                         |
                         +-> promptGuard webhook -> attestation-verifier
```

## Prerequisites

- A kind cluster with enough CPU and memory for the vLLM pod.
- `kubectl`, `helm`, and `kind`.
- The agentgateway controller installed with Inference Extension support enabled.
- Gateway API and Gateway API Inference Extension CRDs installed.

The included vLLM manifest uses the upstream CPU image
`vllm/vllm-openai-cpu:v0.18.1` and the small `facebook/opt-125m` model. If your
kind nodes cannot run that image, override the vLLM image before applying the
manifest.

## Install agentgateway for kind

From the repo root:

```bash
kind create cluster
make -C controller gw-api-crds gie-crds

helm upgrade --install agentgateway-crds \
  controller/install/helm/agentgateway-crds \
  -n agentgateway-system \
  --create-namespace

helm upgrade --install agentgateway \
  controller/install/helm/agentgateway \
  -n agentgateway-system \
  --create-namespace \
  --set inferenceExtension.enabled=true \
  --set image.registry=ghcr.io/agentgateway \
  --set image.tag=v1.0.1-dev \
  --set controller.image.repository=agentgateway-controller \
  --set proxy.image.repository=agentgateway
```

If you are using locally built controller or proxy images, set the corresponding
chart image values and load those images into kind before installing the chart.
The `v1.0.1-dev` tag above matches the local kind build output.

## Deploy the demo

```bash
kubectl apply -f examples/confidential-computing-vllm/attestation-verifier.yaml
kubectl apply -f examples/confidential-computing-vllm/epp.yaml
kubectl apply -f examples/confidential-computing-vllm/vllm-inferencepool.yaml
kubectl apply -f examples/confidential-computing-vllm/agentgateway.yaml
```

Wait for the pods:

```bash
kubectl -n confidential-vllm wait --for=condition=Ready pod \
  -l app.kubernetes.io/part-of=confidential-vllm \
  --timeout=20m
```

Port-forward the generated Gateway service:

```bash
kubectl -n confidential-vllm port-forward svc/confidential-vllm 8080:80
```

## Try it

Without the attestation report header, agentgateway rejects the request before
vLLM is called:

```bash
curl -i http://localhost:8080/v1/chat/completions \
  -H "content-type: application/json" \
  -d '{
    "model": "confidential-vllm",
    "messages": [
      {"role": "user", "content": "Say hello from confidential vLLM"}
    ],
    "max_tokens": 16
  }'
```

With the demo report hash, the verifier allows the request:

```bash
curl -i http://localhost:8080/v1/chat/completions \
  -H "content-type: application/json" \
  -H "x-attestation-report: demo-report-hash" \
  -H "x-attestation-report-url: https://proof.t16z.com/" \
  -d '{
    "model": "confidential-vllm",
    "messages": [
      {"role": "user", "content": "Say hello from confidential vLLM"}
    ],
    "max_tokens": 16
  }'
```

To use a real report from https://proof.t16z.com/, upload the SGX or TDX quote,
copy the report hash, and update the `EXPECTED_REPORT_HASH` environment variable
in `attestation-verifier.yaml`.

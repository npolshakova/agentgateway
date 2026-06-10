# Confidential Computing vLLM Example

This example has two modes:

- [`tee/`](tee/README.md): a confidential-computing deployment where
  agentgateway, vLLM, TLS termination, and attestation all run inside the TEE.
- The Kubernetes manifests in this directory: a kind-based TEE simulator that
  exercises confidential-node placement, agentgateway policy, InferencePool
  routing, and nonce-bound attestation gating locally.

The TEE deployment is the confidential path. The kind setup is intentionally a
simulation: kind on a laptop cannot provide Intel TDX, NVIDIA Confidential
Computing, encrypted guest memory, or TEE-bound TLS keys.

For real confidential LLM inference, agentgateway must run inside the TEE. If
agentgateway runs outside the confidential VM, it can see prompts and responses.
In the TEE deployment, clients first fetch `/attestation`, inspect the TDX quote
with https://proof.t16z.com/, compare the compose hash with the reviewed
workload, and then send prompts over TLS that terminates in agentgateway inside
the TEE.

The kind attestation verifier is demo-only. It mints local, HMAC-signed
attestation-like evidence from `/attestation?nonce=...`, then verifies that
nonce-bound evidence in an agentgateway prompt-guard webhook before the request
reaches vLLM.

The kind flow is:

```text
client -> /attestation -> local TEE simulator evidence
client -> Gateway -> HTTPRoute -> AgentgatewayBackend -> InferencePool -> vLLM
                         |
                         +-> promptGuard webhook verifies simulator evidence
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

`facebook/opt-125m` is only used to keep the kind demo and laptop compose smoke
test small. It is not an instruction-tuned chat model, so its completions may
look odd. The TEE GPU override uses `Qwen/Qwen2.5-7B-Instruct`, which is a
better fit for OpenAI-compatible chat completion demos on GPU hardware.

## Run the TEE Deployment

Use [`tee/docker-compose.yaml`](tee/docker-compose.yaml) on a confidential VM
with `/var/run/dstack.sock` mounted. On a GPU TEE, include the GPU override:

```bash
cd examples/confidential-computing-vllm/tee
docker compose -f docker-compose.yaml -f docker-compose.gpu.yaml up --build
```

Then verify the workload before sending prompts:

```bash
curl -k "https://localhost:8443/attestation?nonce=$(openssl rand -hex 32)"
```

Paste the quote into https://proof.t16z.com/ and compare the returned
`info.compose_hash` with the compose file you reviewed. After verification,
send inference requests through agentgateway:

```bash
curl -k https://localhost:8443/v1/chat/completions \
  -H "content-type: application/json" \
  -d '{
    "model": "confidential-vllm",
    "messages": [
      {"role": "user", "content": "Say hello from confidential vLLM"}
    ]
  }'
```

See [`tee/README.md`](tee/README.md) for the full TEE flow and security notes.

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

These manifests are for the local kind simulation. First label the kind node as
the simulated confidential node:

```bash
kubectl label node kind-control-plane \
  node.agentgateway.dev/confidential-compute=simulated \
  --overwrite
```

In a real confidential Kubernetes cluster, this label would be applied only to
nodes backed by TDX, SEV-SNP, or confidential GPU infrastructure. For the local
demo, it lets Kubernetes show the same scheduling boundary.

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
vLLM is called in the kind simulation:

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
NONCE=$(openssl rand -hex 32)
REPORT=$(curl -s "http://localhost:8080/attestation?nonce=${NONCE}" \
  | python3 -c 'import json,sys; print(json.load(sys.stdin)["report"])')

curl -i http://localhost:8080/v1/chat/completions \
  -H "content-type: application/json" \
  -H "x-attestation-report: ${REPORT}" \
  -H "x-attestation-nonce: ${NONCE}" \
  -H "x-attestation-report-url: https://proof.t16z.com/" \
  -d '{
    "model": "confidential-vllm",
    "messages": [
      {"role": "user", "content": "Say hello from confidential vLLM"}
    ],
    "max_tokens": 16
  }'
```

Inspect the local simulator evidence:

```bash
curl -s "http://localhost:8080/attestation?nonce=$(openssl rand -hex 32)" \
  | python3 -m json.tool
```

The evidence includes the nonce, simulated workload hash, simulated confidential
node selector, expiry, and a local signature. It is intentionally not hardware
evidence.

## Confidential Kubernetes Alternative

When running in Kubernetes:

- Replace the local HMAC attestation simulator with a verifier for real TDX,
  SEV-SNP, or NVIDIA CC evidence.
- Apply the confidential node label only from trusted node admission or node
  provisioning automation.
- Keep the agentgateway proxy, vLLM pod, EPP, and attestation service scheduled
  onto confidential nodes.
- Terminate TLS inside the agentgateway data plane running on the confidential
  node.
- Compare hardware quote measurements and workload hashes with the deployment
  manifests before sending prompts.

For real attestation without Kubernetes, use the TEE deployment in [`tee/`](tee/).

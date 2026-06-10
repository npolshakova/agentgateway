# TEE Deployment

This variant runs agentgateway, vLLM, and attestation inside the same
confidential VM. This is the path that makes the LLM inference confidential:
agentgateway terminates TLS inside the TEE, routes `/v1/*` to vLLM over the
TEE-local network, and exposes `/attestation` so clients can verify the VM
measurement before sending prompts.

```text
client --TLS--> agentgateway --HTTP inside TEE--> vLLM
                    |
                    +--> /attestation --> dstack quote from /var/run/dstack.sock
```

## Run

The base compose file runs the same service layout without requiring a local
NVIDIA GPU. This is useful for smoke-testing the wiring, including TLS
termination in agentgateway, on a laptop:

```bash
cd examples/confidential-computing-vllm/tee
docker compose up --build
```

On a real confidential deployment, use a confidential GPU instance and the GPU
override:

```bash
MODEL_ID=Qwen/Qwen2.5-7B-Instruct \
AGENTGATEWAY_IMAGE=ghcr.io/agentgateway/agentgateway:v1.0.1-dev \
docker compose -f docker-compose.yaml -f docker-compose.gpu.yaml up --build
```

The GPU override requests an NVIDIA device. Do not use it on Docker Desktop for
Mac; use the Kubernetes simulator in the parent directory for local demos.

## Verify Before Sending Prompts

Get a fresh quote with a nonce:

```bash
curl -k "https://localhost:8443/attestation?nonce=$(openssl rand -hex 32)"
```

The response includes:

- `quote`: the TDX attestation quote and event log from dstack.
- `info.compose_hash`: the hash of the reviewed compose workload.
- `report_data_binding`: the caller nonce plus the agentgateway TLS certificate
  fingerprint.
- `proof_url`: https://proof.t16z.com/ for visual quote inspection.

Paste the quote into https://proof.t16z.com/ and compare the compose hash with
the `docker-compose.yaml` you reviewed. The report data binds the quote to the
nonce and the TLS certificate, so a client can reject stale quotes or quotes for
a different endpoint.

## Call vLLM Through agentgateway

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

## Security Notes

- agentgateway must run inside the TEE. If it runs outside, it can see prompts
  and responses.
- TLS must terminate inside the TEE. The `tls-init` service asks dstack for an
  RA-TLS key; outside dstack it falls back to a short-lived self-signed
  certificate only so the files can be smoke-tested.
- vLLM must run inside the TEE. For GPU confidentiality, run on NVIDIA CC-capable
  hardware such as H100, H200, or Blackwell with confidential mode enabled.
- Disk and model cache storage should be encrypted by the TEE platform.
- Attestation proves what is running and on what hardware. It does not prove the
  code is safe; users still need to review this compose file and the images it
  references.

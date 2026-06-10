import base64
import hashlib
import json
import os
import sys
from datetime import datetime, timedelta, timezone
from pathlib import Path

from cryptography import x509
from cryptography.hazmat.primitives import hashes, serialization
from cryptography.hazmat.primitives.asymmetric import rsa
from cryptography.x509.oid import NameOID
from fastapi import FastAPI, HTTPException, Query
from fastapi.responses import JSONResponse

try:
    from dstack_sdk import DstackClient
except Exception:
    DstackClient = None


CERT_DIR = Path("/certs")
CERT_PATH = CERT_DIR / "tls.crt"
KEY_PATH = CERT_DIR / "tls.key"
PROOF_URL = os.environ.get("PROOF_URL", "https://proof.t16z.com/")

app = FastAPI()


def _client():
    if DstackClient is None:
        raise RuntimeError("dstack-sdk is unavailable")
    return DstackClient()


def _jsonable(value):
    if isinstance(value, bytes):
        return base64.b64encode(value).decode("ascii")
    if isinstance(value, Path):
        return str(value)
    if hasattr(value, "model_dump"):
        return _jsonable(value.model_dump())
    if isinstance(value, dict):
        return {k: _jsonable(v) for k, v in value.items()}
    if isinstance(value, list):
        return [_jsonable(v) for v in value]
    return value


def _write_tls_from_dstack():
    tls = _client().get_tls_key(
        subject="confidential-vllm",
        alt_names=["localhost", "agentgateway", "confidential-vllm"],
        usage_ra_tls=True,
        usage_server_auth=True,
    )
    key = tls.key if hasattr(tls, "key") else tls["key"]
    chain = tls.certificate_chain if hasattr(tls, "certificate_chain") else tls["certificate_chain"]
    CERT_DIR.mkdir(parents=True, exist_ok=True)
    KEY_PATH.write_text(key, encoding="utf-8")
    CERT_PATH.write_text("\n".join(chain), encoding="utf-8")


def _write_self_signed_tls():
    key = rsa.generate_private_key(public_exponent=65537, key_size=2048)
    subject = issuer = x509.Name(
        [x509.NameAttribute(NameOID.COMMON_NAME, "confidential-vllm.local")]
    )
    cert = (
        x509.CertificateBuilder()
        .subject_name(subject)
        .issuer_name(issuer)
        .public_key(key.public_key())
        .serial_number(x509.random_serial_number())
        .not_valid_before(datetime.now(timezone.utc))
        .not_valid_after(datetime.now(timezone.utc) + timedelta(days=7))
        .add_extension(
            x509.SubjectAlternativeName(
                [
                    x509.DNSName("localhost"),
                    x509.DNSName("agentgateway"),
                    x509.DNSName("confidential-vllm"),
                ]
            ),
            critical=False,
        )
        .sign(key, hashes.SHA256())
    )
    CERT_DIR.mkdir(parents=True, exist_ok=True)
    KEY_PATH.write_bytes(
        key.private_bytes(
            serialization.Encoding.PEM,
            serialization.PrivateFormat.TraditionalOpenSSL,
            serialization.NoEncryption(),
        )
    )
    CERT_PATH.write_bytes(cert.public_bytes(serialization.Encoding.PEM))


def _tls_fingerprint():
    if not CERT_PATH.exists():
        return None
    cert = x509.load_pem_x509_certificate(CERT_PATH.read_bytes())
    return cert.fingerprint(hashes.SHA256()).hex()


def generate_tls():
    try:
        _write_tls_from_dstack()
        source = "dstack-ra-tls"
    except Exception as err:
        if os.environ.get("REQUIRE_DSTACK_TLS", "").lower() in {"1", "true", "yes"}:
            raise
        _write_self_signed_tls()
        source = f"self-signed-fallback: {err}"
    print(json.dumps({"tls": source, "cert_sha256": _tls_fingerprint()}))


@app.get("/health")
def health():
    return {"status": "ok"}


@app.get("/attestation")
def attestation(nonce: str = Query(..., min_length=8, max_length=128)):
    cert_sha256 = _tls_fingerprint()
    binding = {
        "nonce": nonce,
        "tls_cert_sha256": cert_sha256,
        "service": "agentgateway-confidential-vllm",
    }
    report_data = hashlib.sha512(
        json.dumps(binding, sort_keys=True, separators=(",", ":")).encode("utf-8")
    ).digest()
    try:
        client = _client()
        info = client.info()
        quote = client.get_quote(report_data)
    except Exception as err:
        raise HTTPException(status_code=503, detail=f"dstack attestation unavailable: {err}")

    return JSONResponse(
        {
            "proof_url": PROOF_URL,
            "report_data_binding": binding,
            "report_data_sha512": report_data.hex(),
            "info": _jsonable(info),
            "quote": _jsonable(quote),
        }
    )


if __name__ == "__main__":
    mode = sys.argv[1] if len(sys.argv) > 1 else "serve"
    if mode == "generate-tls":
        generate_tls()
    elif mode == "serve":
        import uvicorn

        uvicorn.run(app, host="0.0.0.0", port=8081)
    else:
        raise SystemExit(f"unknown mode: {mode}")

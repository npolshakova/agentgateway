package remotehttp

import (
	"testing"

	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
)

func TestRequestKeyIncludesTransportSemantics(t *testing.T) {
	t.Parallel()

	strict := FetchTarget{
		URL: "https://issuer.example/jwks",
		Transport: TransportFingerprint{
			CABundleHash: "ca-a",
		},
	}
	hostname := FetchTarget{
		URL: "https://issuer.example/jwks",
		Transport: TransportFingerprint{
			Verification: agentgateway.InsecureTLSModeHostname,
			CABundleHash: "ca-a",
		},
	}
	differentCA := FetchTarget{
		URL: "https://issuer.example/jwks",
		Transport: TransportFingerprint{
			CABundleHash: "ca-b",
		},
	}

	if strict.Key() == hostname.Key() {
		t.Fatalf("expected hostname verification to produce a distinct request key")
	}
	if strict.Key() == differentCA.Key() {
		t.Fatalf("expected different CA bundles to produce a distinct request key")
	}
}

func TestRequestKeyPreservesVerificationFingerprintCompatibility(t *testing.T) {
	t.Parallel()

	url := "https://issuer.example/jwks"

	strict := FetchTarget{
		URL: url,
		Transport: TransportFingerprint{
			CABundleHash: "ca-a",
		},
	}
	hostname := FetchTarget{
		URL: url,
		Transport: TransportFingerprint{
			Verification: agentgateway.InsecureTLSModeHostname,
			CABundleHash: "ca-a",
		},
	}
	insecure := FetchTarget{
		URL: url,
		Transport: TransportFingerprint{
			Verification: agentgateway.InsecureTLSModeAll,
			CABundleHash: "ca-a",
		},
	}

	if strict.Key() != FetchKey("f88aca84338ee9f79f4bfffc38d0ab986b77d9a84c48ab8494320dd674e1360c") {
		t.Fatalf("strict verification fingerprint changed: %s", strict.Key())
	}
	if hostname.Key() != FetchKey("877060d7809ba8ecec793825b9c3a417a86f58b510fe5caef1459fe2ac813f9e") {
		t.Fatalf("hostname verification fingerprint changed: %s", hostname.Key())
	}
	if insecure.Key() != FetchKey("e9f70ae7c1c52f16f8076f83a4b89efa16d6cdcf8df28a642466bcff4aaf2ded") {
		t.Fatalf("insecure verification fingerprint changed: %s", insecure.Key())
	}
}

func TestRequestKeyPreservesPlainHTTPCompatibility(t *testing.T) {
	t.Parallel()

	request := FetchTarget{
		URL: "http://keycloak.default.svc.cluster.local:7080/realms/mcp/protocol/openid-connect/certs",
	}

	if request.Key() != FetchKey("1e7164f878aa33738bc1ee75f61bbdda058a5435b2908ea9c2cd4f7d6d0fb7b4") {
		t.Fatalf("plain HTTP fingerprint changed: %s", request.Key())
	}
}

func TestRequestKeyDistinguishesByProxyURL(t *testing.T) {
	t.Parallel()

	noProxy := FetchTarget{URL: "https://issuer.example/jwks"}
	withProxy := FetchTarget{URL: "https://issuer.example/jwks", ProxyURL: "http://proxy:8080"}
	differentProxy := FetchTarget{URL: "https://issuer.example/jwks", ProxyURL: "http://other-proxy:3128"}

	if noProxy.Key() == withProxy.Key() {
		t.Fatalf("expected proxy URL to produce a distinct request key")
	}
	if withProxy.Key() == differentProxy.Key() {
		t.Fatalf("expected different proxy URLs to produce distinct request keys")
	}
}

func TestRequestKeyPreservesALPNOrder(t *testing.T) {
	t.Parallel()

	first := FetchTarget{
		URL: "https://issuer.example/jwks",
		Transport: TransportFingerprint{
			NextProtos: []string{"h2", "http/1.1"},
		},
	}
	second := FetchTarget{
		URL: "https://issuer.example/jwks",
		Transport: TransportFingerprint{
			NextProtos: []string{"http/1.1", "h2"},
		},
	}

	if first.Key() == second.Key() {
		t.Fatalf("expected ALPN order to produce a distinct request key")
	}
}

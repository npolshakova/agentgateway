package main

import (
	"crypto/tls"
	"crypto/x509"
	"encoding/base64"
	"encoding/json"
	"fmt"
	"net/http"
	"strings"
	"time"

	_ "embed"
)

//go:embed dummy-idp.cert
var cert []byte

//go:embed dummy-idp.key
var key []byte

func startDummyIDP() (shutdownFunc, error) {
	roots := x509.NewCertPool()
	if !roots.AppendCertsFromPEM(cert) {
		return nil, fmt.Errorf("failed to append Cert from PEM")
	}

	cert, err := tls.X509KeyPair(cert, key)
	if err != nil {
		return nil, err
	}

	mux := http.NewServeMux()
	mux.HandleFunc("/org-one/keys", func(w http.ResponseWriter, req *http.Request) {
		w.Header().Add("content-type", "application/json")
		w.Write(orgOneJwks)
	})
	mux.HandleFunc("/org-two/keys", func(w http.ResponseWriter, req *http.Request) {
		w.Header().Add("content-type", "application/json")
		w.Write(orgTwoJwks)
	})
	mux.HandleFunc("/org-three/keys", func(w http.ResponseWriter, req *http.Request) {
		w.Header().Add("content-type", "application/json")
		w.Write(orgThreeJwks)
	})
	mux.HandleFunc("/org-four/keys", func(w http.ResponseWriter, req *http.Request) {
		w.Header().Add("content-type", "application/json")
		w.Write(orgFourJwks)
	})
	mux.HandleFunc("/org-one/jwt", func(w http.ResponseWriter, req *http.Request) {
		w.Header().Add("content-type", "application/json")
		w.Write(orgOneJwt)
	})
	mux.HandleFunc("/org-two/jwt", func(w http.ResponseWriter, req *http.Request) {
		w.Header().Add("content-type", "application/json")
		w.Write(orgTwoJwt)
	})
	mux.HandleFunc("/org-three/jwt", func(w http.ResponseWriter, req *http.Request) {
		w.Header().Add("content-type", "application/json")
		w.Write(orgThreeJwt)
	})
	mux.HandleFunc("/org-four/jwt", func(w http.ResponseWriter, req *http.Request) {
		w.Header().Add("content-type", "application/json")
		w.Write(orgFourJwt)
	})

	// OAuth2/OIDC endpoints
	mux.HandleFunc("/register", handleRegister)
	mux.HandleFunc("/authorize", handleAuthorize)
	mux.HandleFunc("/token", handleToken)
	// Handle .well-known paths - register each path explicitly
	mux.HandleFunc("/.well-known/jwks.json", handleJWKS)
	mux.HandleFunc("/.well-known/oauth-authorization-server", handleDiscovery)

	// Add CORS middleware for all routes
	muxWithCORS := http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.Method == http.MethodOptions {
			handleOPTIONS(w, r)
			return
		}
		mux.ServeHTTP(w, r)
	})

	// nolint: gosec // Test code only
	cfg := &tls.Config{
		RootCAs:      roots,
		Certificates: []tls.Certificate{cert},
		NextProtos:   []string{"http/1.1"},
	}

	// nolint: gosec // Test code only
	srv := &http.Server{
		Addr:         "0.0.0.0:8443",
		Handler:      muxWithCORS,
		TLSConfig:    cfg,
		TLSNextProto: make(map[string]func(*http.Server, *tls.Conn, http.Handler), 0),
	}

	return serveHTTP("dummy-idp", srv, func() error {
		return srv.ListenAndServeTLS("", "")
	}), nil
}

// OAuth2/OIDC constants
const (
	hardcodedClientID = "mcp_gi3APARn2_uHv2oxfJJqq2yZBDV4OyNo"
	hardcodedCode     = "fixed_auth_code_123"
	// nolint: gosec // Test code only
	hardcodedClientSecret = "secret_2nGx_bjvo9z72Aw3-hKTWMusEo2-yTfH"
	hardcodedRefreshToken = "fixed_refresh_token_123"
	redirectURI           = "http://localhost:8081/callback"
)

// sendJSONResponse sends a JSON response with CORS headers
func sendJSONResponse(w http.ResponseWriter, r *http.Request, data any, statusCode int) {
	w.Header().Set("Content-Type", "application/json")
	origin := r.Header.Get("Origin")
	if origin == "" {
		origin = "*"
	}
	w.Header().Set("Access-Control-Allow-Origin", origin)
	w.Header().Set("Vary", "Origin")
	w.Header().Set("Access-Control-Allow-Credentials", "true")
	requestHeaders := r.Header.Get("Access-Control-Request-Headers")
	if requestHeaders == "" {
		requestHeaders = "content-type, authorization"
	}
	w.Header().Set("Access-Control-Allow-Headers", requestHeaders)
	w.WriteHeader(statusCode)
	json.NewEncoder(w).Encode(data)
}

// handleRegister handles OAuth2 client registration
func handleRegister(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodPost {
		sendJSONResponse(w, r, map[string]string{"error": "method_not_allowed"}, http.StatusMethodNotAllowed)
		return
	}

	registration := map[string]any{
		"client_id":                  hardcodedClientID,
		"client_secret":              hardcodedClientSecret,
		"client_name":                "Test Client",
		"client_description":         "A test MCP client",
		"redirect_uris":              []string{redirectURI},
		"grant_types":                []string{"authorization_code", "refresh_token"},
		"response_types":             []string{"code"},
		"token_endpoint_auth_method": "client_secret_basic",
		"created_at":                 time.Now().Format(time.RFC3339Nano),
		"updated_at":                 time.Now().Format(time.RFC3339Nano),
	}
	sendJSONResponse(w, r, registration, http.StatusOK)
}

// handleAuthorize handles OAuth2 authorization endpoint
func handleAuthorize(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodGet {
		sendJSONResponse(w, r, map[string]string{"error": "method_not_allowed"}, http.StatusMethodNotAllowed)
		return
	}

	query := r.URL.Query()
	clientID := query.Get("client_id")
	redirectURI := query.Get("redirect_uri")

	if clientID != hardcodedClientID {
		sendJSONResponse(w, r, map[string]string{"error": "invalid_client"}, http.StatusBadRequest)
		return
	}

	callbackURL := fmt.Sprintf("%s?code=%s", redirectURI, hardcodedCode)
	sendJSONResponse(w, r, map[string]string{"redirect_to": callbackURL}, http.StatusOK)
}

// handleToken handles OAuth2 token endpoint
func handleToken(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodPost {
		sendJSONResponse(w, r, map[string]string{"error": "method_not_allowed"}, http.StatusMethodNotAllowed)
		return
	}

	if err := r.ParseForm(); err != nil {
		sendJSONResponse(w, r, map[string]string{"error": "invalid_request"}, http.StatusBadRequest)
		return
	}

	grantType := r.FormValue("grant_type")
	clientID := r.FormValue("client_id")
	clientSecret := r.FormValue("client_secret")

	// Extract Basic auth header if client_id not in body
	authHeader := r.Header.Get("Authorization")
	if clientID == "" && strings.HasPrefix(authHeader, "Basic ") {
		decoded, err := base64.StdEncoding.DecodeString(strings.TrimPrefix(authHeader, "Basic "))
		if err == nil {
			parts := strings.SplitN(string(decoded), ":", 2)
			if len(parts) == 2 {
				clientID = parts[0]
				clientSecret = parts[1]
			}
		}
	}

	switch grantType {
	case "authorization_code":
		// Be lenient for generic MCP inspectors/SPAs using PKCE:
		// - Do not require client_secret (public client)
		// - Accept any code/redirect_uri/code_verifier
		response := map[string]any{
			"access_token":  string(orgOneJwt),
			"refresh_token": hardcodedRefreshToken,
			"token_type":    "bearer",
			"expires_in":    3600,
		}
		sendJSONResponse(w, r, response, http.StatusOK)

	case "refresh_token":
		// For refresh token, still require confidential client auth
		if clientID != hardcodedClientID || clientSecret != hardcodedClientSecret {
			sendJSONResponse(w, r, map[string]string{"error": "invalid_client"}, http.StatusBadRequest)
			return
		}
		// Accept any refresh_token for testing purposes
		response := map[string]any{
			"access_token":  string(orgOneJwt),
			"refresh_token": hardcodedRefreshToken,
			"token_type":    "bearer",
			"expires_in":    3600,
		}
		sendJSONResponse(w, r, response, http.StatusOK)

	default:
		sendJSONResponse(w, r, map[string]string{"error": "unsupported_grant_type"}, http.StatusBadRequest)
	}
}

// handleJWKS handles JWKS endpoint using orgOneJwks
func handleJWKS(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodGet {
		sendJSONResponse(w, r, map[string]string{"error": "method_not_allowed"}, http.StatusMethodNotAllowed)
		return
	}
	// Set CORS headers
	origin := r.Header.Get("Origin")
	if origin == "" {
		origin = "*"
	}
	w.Header().Set("Access-Control-Allow-Origin", origin)
	w.Header().Set("Vary", "Origin")
	w.Header().Set("Access-Control-Allow-Credentials", "true")
	w.Header().Set("Content-Type", "application/json")
	w.WriteHeader(http.StatusOK)
	w.Write(orgOneJwks)
}

// handleDiscovery handles OAuth2 discovery endpoint
func handleDiscovery(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodGet {
		sendJSONResponse(w, r, map[string]string{"error": "method_not_allowed"}, http.StatusMethodNotAllowed)
		return
	}

	// Determine base URL from request
	scheme := "https"
	if r.TLS == nil {
		scheme = "http"
	}
	host := r.Host
	if host == "" {
		host = "localhost:8443"
	}
	baseURL := fmt.Sprintf("%s://%s", scheme, host)

	discovery := map[string]any{
		"issuer":                                "https://agentgateway.dev",
		"authorization_endpoint":                fmt.Sprintf("%s/authorize", baseURL),
		"token_endpoint":                        fmt.Sprintf("%s/token", baseURL),
		"jwks_uri":                              fmt.Sprintf("%s/.well-known/jwks.json", baseURL),
		"registration_endpoint":                 fmt.Sprintf("%s/register", baseURL),
		"response_types_supported":              []string{"code"},
		"grant_types_supported":                 []string{"authorization_code", "refresh_token"},
		"token_endpoint_auth_methods_supported": []string{"none", "client_secret_basic", "client_secret_post"},
		"code_challenge_methods_supported":      []string{"S256"},
	}
	sendJSONResponse(w, r, discovery, http.StatusOK)
}

// handleOPTIONS handles CORS preflight requests
func handleOPTIONS(w http.ResponseWriter, r *http.Request) {
	origin := r.Header.Get("Origin")
	if origin == "" {
		origin = "*"
	}
	requestHeaders := r.Header.Get("Access-Control-Request-Headers")
	if requestHeaders == "" {
		requestHeaders = "content-type"
	}

	w.Header().Set("Access-Control-Allow-Origin", origin)
	w.Header().Set("Vary", "Origin")
	w.Header().Set("Access-Control-Allow-Methods", "GET, POST, OPTIONS")
	w.Header().Set("Access-Control-Allow-Headers", requestHeaders)
	w.Header().Set("Access-Control-Allow-Credentials", "true")
	w.WriteHeader(http.StatusNoContent)
}

var (
	// jwks and jwts were generated using hack/utils/jwt/jwt-generator.go
	// jwts are valid until Aug 2035
	//   "iss": "https://agentgateway.dev",
	//   "sub": "ignore@agentgateway.dev",
	orgOneJwks = []byte(`{"keys":[{"use":"sig","kty":"RSA","kid":"5350231219306038692","n":"nZPFlqxzFp6fpDjtBV4mj9DDqgD2VEm3Ji4cFe99IKBk2B5hT8RFDXHahLwxmUSHcgZkY1cZW167pByxBAL69xqiGhbTDt0LuvKiRo4wysDP_Vod28Pmnh1mCdXxlweH4iDHyjPmEV3bh6AqlDAPX0ZvT3pZnzoVkBIAYeP00_Xo6fUleVMq-b7u6CRbhEX4xdQug7VGd5ZwE2vlWOARAAkaQj0XY6Kz6EHGi1PY5yzHz9hIZhWo0qA9CZ_XIyA12J9ICNFoEpqwCzeSJOeh6jJgPaCQbRe4lBDeHJFa4SKSR_Imau--MpWcN7_2JZ72HUmZRU-9aIhmYkZtdfjwXw","e":"AQAB","x5c":["MIIC3jCCAcagAwIBAgIBITANBgkqhkiG9w0BAQsFADAXMRUwEwYDVQQKEwxrZ2F0ZXdheS5kZXYwHhcNMjUxMTE5MTkxMDA3WhcNMjUxMTE5MjExMDA3WjAXMRUwEwYDVQQKEwxrZ2F0ZXdheS5kZXYwggEiMA0GCSqGSIb3DQEBAQUAA4IBDwAwggEKAoIBAQCdk8WWrHMWnp+kOO0FXiaP0MOqAPZUSbcmLhwV730goGTYHmFPxEUNcdqEvDGZRIdyBmRjVxlbXrukHLEEAvr3GqIaFtMO3Qu68qJGjjDKwM/9Wh3bw+aeHWYJ1fGXB4fiIMfKM+YRXduHoCqUMA9fRm9PelmfOhWQEgBh4/TT9ejp9SV5Uyr5vu7oJFuERfjF1C6DtUZ3lnATa+VY4BEACRpCPRdjorPoQcaLU9jnLMfP2EhmFajSoD0Jn9cjIDXYn0gI0WgSmrALN5Ik56HqMmA9oJBtF7iUEN4ckVrhIpJH8iZq774ylZw3v/YlnvYdSZlFT71oiGZiRm11+PBfAgMBAAGjNTAzMA4GA1UdDwEB/wQEAwIFoDATBgNVHSUEDDAKBggrBgEFBQcDATAMBgNVHRMBAf8EAjAAMA0GCSqGSIb3DQEBCwUAA4IBAQA8ZNw+i8b1mvbPfRXyez2t0B68Eodg+OO2Dki4WTPtIgQaTrC3vHRyHrol479Mmete+3F00NRqfT8Fo06MVbLXv1Zv1d+JQjJmcy4tyVyBm+pKqYXBxuhEIdBmzXGIV36vyZ1rFcm9O81k0OouBVbpKn0JGbpXR4P9GBn50G26lmqBsMIsQ3K0zJl7b9vlVgvZeV4RPBWUTAK9F4LdwrB3NeEdRcI4ri91PfwgOoPe2h3rUcfCb+XSl9tqgrfkX2Gt0H3PCRgre+XdOAwNHaVhrxxWrkacTAK8oQdftBKLiRVsEMqXmV4PpayB0PxEGDDa+XYmEKuF8br4Z+MgFdsJ"]}]}`)
	orgOneJwt  = []byte(`eyJhbGciOiJSUzI1NiIsImtpZCI6IjUzNTAyMzEyMTkzMDYwMzg2OTIiLCJ0eXAiOiJKV1QifQ.eyJpc3MiOiJodHRwczovL2FnZW50Z2F0ZXdheS5kZXYiLCJzdWIiOiJpZ25vcmVAYWdlbnRnYXRld2F5LmRldiIsImV4cCI6MjA3MTE2MzQwNywibmJmIjoxNzc3NzM1MTQ3LCJpYXQiOjE3Nzc3MzUxNDd9.lNeg5hUvY7SaqMcOM8hH-Ji13-1qUbwKJJ4oz8n2mnf3r99fIvErwtvJeRSs8zey_RqAl77aY72kc7q6zbY0p5R5neOQgk68fsZ7l56nM2ErXjDKKgq8e61zgk4wW1VHL7RFMAvHkFklXubj4W6RxCl2rxIm4jNHZYT_a4kGh67PUEZvhrAGDcB0xYfG0rj-x3hAa4dwpD7-1PWt16KeSEMVsUmnhvnvLwRJbsFkm1vlAC6JqSYLm4Jx4Fp-oZf9w0o59O319xGtQUbcnHQ3ZUsM2vdyCNIbOuGJs2RX08xAhrvRJ3nORyb3cvF3VaIqVswErslGpCHedeRGK0ykYlSlL_HnEyYagWuMlmYNQz9L3I-jAoeGzqQu9EO-_VN7obgVOp1CVX7lTJpeOQbUXcs0xGXHuPXYwp0GBLnapayvzN8l_Q845EsaXGuMvH3QwfjrPqGMpv7Xd_rd5VdkJfzJcpEDchJQ9gk8zGf7p8OWNPWc2WxxiBdvblKzA1s2qcszzCdJasfYY3JqExL4_uytuy1gzE7MMg0tP7zCqYfIBxWSWkhPFBeu702BPdbsFyaH3Hd9P7rf7y8pDHMo1JRbbRNtON0Q90y5mno2bsS2vfKjIpFlY97XXSj8LS-Vg6vCRyP9n490dHyCOfuuwehxiuivRjNeHpBaQaIf2mE`)

	orgTwoJwks = []byte(`{"keys":[{"use":"sig","kty":"RSA","kid":"1678034842627949163","n":"xT-a_qwjD_RIeHtDx6V_0LkHbmu6a3C-rmYK8BHU-fmgdU8gfMqkVvtdzHSwLlsR0yMesIYx2JeSuqwThTEb3MCBuepuUDOvvQHKF57XSwp1odRKZGzjZSXHNTMny5ioxBaVphsYT5pGFe-znmFhqbpeh96txTwpIP_oc3w_Ioc7gujx6oCvLddY-bACgIyq1kFsGGh6-dCfkXt4XDg6ZJLZ-9TMCkKIPRATPJ2eDetkAnRSD_nI_PCfK6yemfJZwAvU_-Le6Y-Phr3Fsnj0VaU7M-ANilPixzwDHPxpzhbk1r_TWpuroRRP8yIY2nnoaeSWARDzzUr4Bzv8dBiDxQ","e":"AQAB","x5c":["MIIC5zCCAc+gAwIBAgICAIswDQYJKoZIhvcNAQELBQAwGzEZMBcGA1UEChMQYWdlbnRnYXRld2F5LmRldjAeFw0yNjA1MDIxNzIxMjlaFw0yNjA1MDIxOTIxMjlaMBsxGTAXBgNVBAoTEGFnZW50Z2F0ZXdheS5kZXYwggEiMA0GCSqGSIb3DQEBAQUAA4IBDwAwggEKAoIBAQDFP5r+rCMP9Eh4e0PHpX/QuQdua7prcL6uZgrwEdT5+aB1TyB8yqRW+13MdLAuWxHTIx6whjHYl5K6rBOFMRvcwIG56m5QM6+9AcoXntdLCnWh1EpkbONlJcc1MyfLmKjEFpWmGxhPmkYV77OeYWGpul6H3q3FPCkg/+hzfD8ihzuC6PHqgK8t11j5sAKAjKrWQWwYaHr50J+Re3hcODpkktn71MwKQog9EBM8nZ4N62QCdFIP+cj88J8rrJ6Z8lnAC9T/4t7pj4+GvcWyePRVpTsz4A2KU+LHPAMc/GnOFuTWv9Nam6uhFE/zIhjaeehp5JYBEPPNSvgHO/x0GIPFAgMBAAGjNTAzMA4GA1UdDwEB/wQEAwIFoDATBgNVHSUEDDAKBggrBgEFBQcDATAMBgNVHRMBAf8EAjAAMA0GCSqGSIb3DQEBCwUAA4IBAQBs5/kIfNIqqH0F9btBrGAPdNV+TApi3RXtHeV1CGNSO8n/pqn1qf9YvdvOAHLFXzrr3xKPat5iGOogOvpbCyh0BDmFgF0z1LvEymYzoQsfTGgFIWh0CakvLjyCxc9norSkcrMhUSKnbWEWknRP15ZAsbToiV9KBKcRI7t5M0pz108NtX2DvdlIiocWNgTij4M00SD2jmPuKn2PJBUI/3wbBAmDtfPA3rB673knqU8aafG1LHMuDoVEtjvTr3S48YUGsqqbBxr8sUcMVIzWiuhmDCbpFdrd/6ZCSFqeTB6Mo3co94HDEyPHtob3i8UhyuKT+KPcPpOkpkJ+p53qYYl4"]}]}`)
	orgTwoJwt  = []byte(`eyJhbGciOiJSUzI1NiIsImtpZCI6IjE2NzgwMzQ4NDI2Mjc5NDkxNjMiLCJ0eXAiOiJKV1QifQ.eyJpc3MiOiJodHRwczovL2FnZW50Z2F0ZXdheS5kZXYiLCJzdWIiOiJpZ25vcmVAYWdlbnRnYXRld2F5LmRldiIsImV4cCI6MjA5MzEwMjQ4OSwibmJmIjoxNzc3NzQyNDg5LCJpYXQiOjE3Nzc3NDI0ODl9.WhBDsbQqL5NR7aEODk5FKP2mcBfDIGeGQYakEqkgbfh8YupbO4x8RUYAhbgrG7yIsqZnivYQDC-nWH0gh77wHe0KQ-txJcv_kHWAdgCUFuFVySoREiPrxIhgMTVSD6vtg8Wrksi4UPc07ebUj8RM8uujzaJDWvaSJJbooIqT71K5369MSJ_UoNFKq4hIIi-mMLI0gO0hQeNZAM4Yu0ORDeaLnS1jMg7gdLM12qAkpImxWe-GeaMQuNY6zYCZkR_uDLdKuQFEkFeCIyIXzD_lV2tLMKdLfrTktgYK5lnqRDeOUNJAYKSVYHuIHhHK5WlrT1LjzhkRzVXIiI-QNa7LNQ`)

	orgThreeJwks = []byte(`{"keys":[{"use":"sig","kty":"RSA","kid":"4568088902042034188","n":"twn-c9Plqk4RxmraBR0STCvhbbPnCZPVGoZaKNC3cdI7aGqX1Vn1UhgmP9TNYy1R1B1GP84oed8z-O2mmaa5Nh3v2Ovxlt0vAEOjgerwtLJckmFdzkBOgnqHshGEFaA33qsw7LyDXwbS5taaSO0oLZ24pxxt7ONpxEr83YePrIJQG4LpfCbxbge5J-QOcqrImcyF6S2iUweaHb0m6sILmzSzCS4JBeHhLyHlvGAxJAuxp_ocTzGzlF0E4WUy9kDahUpTx6IqqMqTZSmZOUh0NLDGD9GXn7T9tIowlnDYA2iO5hBuQVcOqOg1yiB9iKfHqmrjhfjUTYHSLUOTiwXeKQ","e":"AQAB","x5c":["MIIC5jCCAc6gAwIBAgIBbjANBgkqhkiG9w0BAQsFADAbMRkwFwYDVQQKExBhZ2VudGdhdGV3YXkuZGV2MB4XDTI2MDUwMjE3MjEzMFoXDTI2MDUwMjE5MjEzMFowGzEZMBcGA1UEChMQYWdlbnRnYXRld2F5LmRldjCCASIwDQYJKoZIhvcNAQEBBQADggEPADCCAQoCggEBALcJ/nPT5apOEcZq2gUdEkwr4W2z5wmT1RqGWijQt3HSO2hql9VZ9VIYJj/UzWMtUdQdRj/OKHnfM/jtppmmuTYd79jr8ZbdLwBDo4Hq8LSyXJJhXc5AToJ6h7IRhBWgN96rMOy8g18G0ubWmkjtKC2duKccbezjacRK/N2Hj6yCUBuC6Xwm8W4HuSfkDnKqyJnMhektolMHmh29JurCC5s0swkuCQXh4S8h5bxgMSQLsaf6HE8xs5RdBOFlMvZA2oVKU8eiKqjKk2UpmTlIdDSwxg/Rl5+0/bSKMJZw2ANojuYQbkFXDqjoNcogfYinx6pq44X41E2B0i1Dk4sF3ikCAwEAAaM1MDMwDgYDVR0PAQH/BAQDAgWgMBMGA1UdJQQMMAoGCCsGAQUFBwMBMAwGA1UdEwEB/wQCMAAwDQYJKoZIhvcNAQELBQADggEBAGmvJgsma+/SbC/VObYfEnRx/ushXLB6ufsTP+54HCvbfehykJyAh3wi3uyTzTvBXTmJMCaZ7Oe+kp/oobsjToc9wSf154JDIEwz+j/r24QtWv3Uofq3strjmTqQDuXXZFSVlZqxJBeGAuEZ0+Y+9Szh2NEMwpBgCgPRhtCG+2u0TzE+SduyKweJL2MQB6oY02Lsd1hdIzAwEMu86Y6/SMCNIrJGpXc/7djMOF/my55Cu0EbQhW54Dqf2Mk07UqVBn2368NmOZJfztu8f7rPbmleH7d/Lx73YK9Ti8TqWf0SoTlN1F2Sd+v9kTd/3XydNpJVlgF2dtdpF5FngBuUtbc="]}]}`)
	orgThreeJwt  = []byte(`eyJhbGciOiJSUzI1NiIsImtpZCI6IjQ1NjgwODg5MDIwNDIwMzQxODgiLCJ0eXAiOiJKV1QifQ.eyJpc3MiOiJodHRwczovL2FnZW50Z2F0ZXdheS5kZXYiLCJzdWIiOiJpZ25vcmVAYWdlbnRnYXRld2F5LmRldiIsImV4cCI6MjA5MzEwMjQ5MCwibmJmIjoxNzc3NzQyNDkwLCJpYXQiOjE3Nzc3NDI0OTB9.VUjtqJWFp-LjgMyc4prfN-YTSQpkIcLuswQmYt7mXkN04boZuBrE7t5BdRsXv-vOtqZe7H_c5rZ4Cp7s8szfULlVJAsDogoI-iCpMxxm_yLhcNdREVV9QWL-p5kId276RGmHq_7ytM6hFS2rUO2waEg6TkHt0eiEQK5a2gg2uTdKqnXv9qWzNGL6l6TOZ7qPfnIbvcwBPDMXLzWGA_35F-SHQ6IEXZQnRo2QEroEYVHWlJHGRpewVe5aeFpptTl3tx7SA29WEOShTXFEcdNdTtirPe2vJhqFdzkiERiA5cj5XV-mpwDtlQKfFYU0UfEqA64YZVWH46QRqKNeCxwxYQ`)

	orgFourJwks = []byte(`{"keys":[{"use":"sig","kty":"RSA","kid":"5541011827887185681","n":"rtwSwCpfOidQIXX2Pi2KVdez1LRUYPElYE8sxpM9M5_nddMBafOLmwYajl7v67xCkcKjHCI0zrDj8jyTYBWlEwaxgzJ-Zfe0VbOQGVeABB0UuBBSPmJaOwbS8KNEDjkuKjd9ojg0tThZYMSAs0rmmKF4wh7Jp0_j4aaoRbO3MgAPrU0DKGPHfyiLizN4M9PUKmI378TCIhGOWtO0QtWJNacYz1tRACkh5Jnp_nHINPlLuBIfYpkJbEeAqYY9vA-Y1Js_evNmK5cGyVGkV5Dg9vaQVIGZhtLtqB1IQHDArFr3n02RkjGKbpG0gIZESZzLNhz6x1LVh-a3rxbtluXI_Q","e":"AQAB","x5c":["MIIC5zCCAc+gAwIBAgICAoAwDQYJKoZIhvcNAQELBQAwGzEZMBcGA1UEChMQYWdlbnRnYXRld2F5LmRldjAeFw0yNjA1MDIxNzIxMzBaFw0yNjA1MDIxOTIxMzBaMBsxGTAXBgNVBAoTEGFnZW50Z2F0ZXdheS5kZXYwggEiMA0GCSqGSIb3DQEBAQUAA4IBDwAwggEKAoIBAQCu3BLAKl86J1AhdfY+LYpV17PUtFRg8SVgTyzGkz0zn+d10wFp84ubBhqOXu/rvEKRwqMcIjTOsOPyPJNgFaUTBrGDMn5l97RVs5AZV4AEHRS4EFI+Ylo7BtLwo0QOOS4qN32iODS1OFlgxICzSuaYoXjCHsmnT+PhpqhFs7cyAA+tTQMoY8d/KIuLM3gz09QqYjfvxMIiEY5a07RC1Yk1pxjPW1EAKSHkmen+ccg0+Uu4Eh9imQlsR4Cphj28D5jUmz9682YrlwbJUaRXkOD29pBUgZmG0u2oHUhAcMCsWvefTZGSMYpukbSAhkRJnMs2HPrHUtWH5revFu2W5cj9AgMBAAGjNTAzMA4GA1UdDwEB/wQEAwIFoDATBgNVHSUEDDAKBggrBgEFBQcDATAMBgNVHRMBAf8EAjAAMA0GCSqGSIb3DQEBCwUAA4IBAQAwwpk7pgYtoFcL5Md8Xq0wJ7Bech4ZqQ6XWLC29MhpGmjAitqMW/uYLMUcz//0xDIbDfuND2YenuUahU19Go1pn2pQpQoPoHyQVYVL7DsoY46MrqpF0nTCzM7eS197Ass01Qz6FfL67KiaqP0P+ro4FkoFSfizTicyY6NqbHNpYJNu1PT0gedmiEKgOYG1KQ/hwAHd45UzPxocRxkZvUnpL2Lf3Sh8kt/IW8o+/du8VLFJB2mrm9Il5IgA3kxYctiErDPHovHEBdclySVWyNw2j23Ab9j7Ay6IvlrEJSSaewLfnPXmkiF3wjb6HwKfpD04/EDyjsVVnDchhSUHUVy5"]}]}`)
	// "sub": "boom@agentgateway.dev",
	orgFourJwt = []byte(`eyJhbGciOiJSUzI1NiIsImtpZCI6IjU1NDEwMTE4Mjc4ODcxODU2ODEiLCJ0eXAiOiJKV1QifQ.eyJpc3MiOiJodHRwczovL2FnZW50Z2F0ZXdheS5kZXYiLCJzdWIiOiJib29tQGFnZW50Z2F0ZXdheS5kZXYiLCJleHAiOjIwOTMxMDI0OTAsIm5iZiI6MTc3Nzc0MjQ5MCwiaWF0IjoxNzc3NzQyNDkwfQ.WbOQtmlgLR3oKiYpFgYHwrp1lBs6-rpO4I8gdXTfybi8DzZYCKvkx_Wj5qHQNrYoBNiuOI1Gx1aqg4q_M65wUwCL5I0xamZdoojda-gNkKBWJj6ebyBtNJSRCf3XIyuqXsphvV0uDWhpq7Y2JgBplSWqOXWftKmhShENMqVtzEBCuJl_a8CKMUun7_JYvB99kvzWlg4Jxe18oBZFpSrIBT2_INSA9Rgqk8TSFI8IYokj4BCr6pi1uvVq3qyEdpnkj8VfBQ_Ti5-rsfHghXz0bThe5i7i-TcPRrlxQzCDLLJBm19YLImpKH5M_yjOvmoIwOi23O7d9hn9EBP0hKJccg`)
)

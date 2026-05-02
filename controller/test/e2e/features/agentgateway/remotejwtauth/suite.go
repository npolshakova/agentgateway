//go:build e2e

package remotejwtauth

import (
	"context"
	"net/http"
	"path/filepath"

	"github.com/stretchr/testify/suite"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/types"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/agentgateway/agentgateway/controller/pkg/utils/fsutils"
	"github.com/agentgateway/agentgateway/controller/pkg/utils/requestutils/curl"
	"github.com/agentgateway/agentgateway/controller/test/e2e"
	"github.com/agentgateway/agentgateway/controller/test/e2e/common"
	"github.com/agentgateway/agentgateway/controller/test/e2e/tests/base"
	testmatchers "github.com/agentgateway/agentgateway/controller/test/gomega/matchers"
)

//
// Use `go run hack/utils/jwt/jwt-generator.go`
// to generate jwks and a jwt signed by the key in it
//

var _ e2e.NewSuiteFunc = NewTestingSuite

const (
	namespace = "agentgateway-base"
	// jwt subject is "ignore@agentgateway.dev"
	// could also retrieve these jwts from  https://dummy-idp.default:8443/org-one/jwt, https://dummy-idp.default:8443/org-two/jwt
	JwtOrgOne = "eyJhbGciOiJSUzI1NiIsImtpZCI6IjUzNTAyMzEyMTkzMDYwMzg2OTIiLCJ0eXAiOiJKV1QifQ.eyJpc3MiOiJodHRwczovL2FnZW50Z2F0ZXdheS5kZXYiLCJzdWIiOiJpZ25vcmVAYWdlbnRnYXRld2F5LmRldiIsImV4cCI6MjA3MTE2MzQwNywibmJmIjoxNzc3NzM1MTQ3LCJpYXQiOjE3Nzc3MzUxNDd9.lNeg5hUvY7SaqMcOM8hH-Ji13-1qUbwKJJ4oz8n2mnf3r99fIvErwtvJeRSs8zey_RqAl77aY72kc7q6zbY0p5R5neOQgk68fsZ7l56nM2ErXjDKKgq8e61zgk4wW1VHL7RFMAvHkFklXubj4W6RxCl2rxIm4jNHZYT_a4kGh67PUEZvhrAGDcB0xYfG0rj-x3hAa4dwpD7-1PWt16KeSEMVsUmnhvnvLwRJbsFkm1vlAC6JqSYLm4Jx4Fp-oZf9w0o59O319xGtQUbcnHQ3ZUsM2vdyCNIbOuGJs2RX08xAhrvRJ3nORyb3cvF3VaIqVswErslGpCHedeRGK0ykYlSlL_HnEyYagWuMlmYNQz9L3I-jAoeGzqQu9EO-_VN7obgVOp1CVX7lTJpeOQbUXcs0xGXHuPXYwp0GBLnapayvzN8l_Q845EsaXGuMvH3QwfjrPqGMpv7Xd_rd5VdkJfzJcpEDchJQ9gk8zGf7p8OWNPWc2WxxiBdvblKzA1s2qcszzCdJasfYY3JqExL4_uytuy1gzE7MMg0tP7zCqYfIBxWSWkhPFBeu702BPdbsFyaH3Hd9P7rf7y8pDHMo1JRbbRNtON0Q90y5mno2bsS2vfKjIpFlY97XXSj8LS-Vg6vCRyP9n490dHyCOfuuwehxiuivRjNeHpBaQaIf2mE"
	jwtOrgTwo = "eyJhbGciOiJSUzI1NiIsImtpZCI6IjE2NzgwMzQ4NDI2Mjc5NDkxNjMiLCJ0eXAiOiJKV1QifQ.eyJpc3MiOiJodHRwczovL2FnZW50Z2F0ZXdheS5kZXYiLCJzdWIiOiJpZ25vcmVAYWdlbnRnYXRld2F5LmRldiIsImV4cCI6MjA5MzEwMjQ4OSwibmJmIjoxNzc3NzQyNDg5LCJpYXQiOjE3Nzc3NDI0ODl9.WhBDsbQqL5NR7aEODk5FKP2mcBfDIGeGQYakEqkgbfh8YupbO4x8RUYAhbgrG7yIsqZnivYQDC-nWH0gh77wHe0KQ-txJcv_kHWAdgCUFuFVySoREiPrxIhgMTVSD6vtg8Wrksi4UPc07ebUj8RM8uujzaJDWvaSJJbooIqT71K5369MSJ_UoNFKq4hIIi-mMLI0gO0hQeNZAM4Yu0ORDeaLnS1jMg7gdLM12qAkpImxWe-GeaMQuNY6zYCZkR_uDLdKuQFEkFeCIyIXzD_lV2tLMKdLfrTktgYK5lnqRDeOUNJAYKSVYHuIHhHK5WlrT1LjzhkRzVXIiI-QNa7LNQ"
	// sub "boom@agentgateway.dev"
	jwtOrgFour = "eyJhbGciOiJSUzI1NiIsImtpZCI6IjU1NDEwMTE4Mjc4ODcxODU2ODEiLCJ0eXAiOiJKV1QifQ.eyJpc3MiOiJodHRwczovL2FnZW50Z2F0ZXdheS5kZXYiLCJzdWIiOiJib29tQGFnZW50Z2F0ZXdheS5kZXYiLCJleHAiOjIwOTMxMDI0OTAsIm5iZiI6MTc3Nzc0MjQ5MCwiaWF0IjoxNzc3NzQyNDkwfQ.WbOQtmlgLR3oKiYpFgYHwrp1lBs6-rpO4I8gdXTfybi8DzZYCKvkx_Wj5qHQNrYoBNiuOI1Gx1aqg4q_M65wUwCL5I0xamZdoojda-gNkKBWJj6ebyBtNJSRCf3XIyuqXsphvV0uDWhpq7Y2JgBplSWqOXWftKmhShENMqVtzEBCuJl_a8CKMUun7_JYvB99kvzWlg4Jxe18oBZFpSrIBT2_INSA9Rgqk8TSFI8IYokj4BCr6pi1uvVq3qyEdpnkj8VfBQ_Ti5-rsfHghXz0bThe5i7i-TcPRrlxQzCDLLJBm19YLImpKH5M_yjOvmoIwOi23O7d9hn9EBP0hKJccg"
)

var (
	setup = base.TestCase{
		Manifests: []string{
			getTestFile("common.yaml"),
		},
	}

	testCases = map[string]*base.TestCase{
		"TestRoutePolicySvc": {
			Manifests: []string{secureRoutePolicyManifestSvc},
		},
		"TestRoutePolicySvcCaCert": {
			Manifests: []string{secureRoutePolicyManifestSvcCaCert},
		},
		"TestRoutePolicyBackend": {
			Manifests: []string{insecureRouteManifest, secureRoutePolicyManifestBackend},
		},
		"TestRoutePolicyBackendAndTlsPolicy": {
			Manifests: []string{secureRoutePolicyManifestBackendAndTlsPolicy},
		},
		"TestRoutePolicyWithRbac": {
			Manifests: []string{secureRoutePolicyWithRbacManifest},
		},
		"TestGatewayPolicySvc": {
			Manifests: []string{secureGWPolicyManifestSvc},
		},
		"TestGatewayPolicySvcCaCert": {
			Manifests: []string{secureGWPolicyManifestSvcCaCert},
		},
		"TestGatewayPolicyBackend": {
			Manifests: []string{secureGWPolicyManifestBackend},
		},
		"TestGatewayPolicyBackendWithTlsPolicy": {
			Manifests: []string{secureGWPolicyManifestBackendAndTlsPolicy},
		},
		"TestGatewayPolicyWithRbac": {
			Manifests: []string{secureGWPolicyWithRbacManifest},
		},
	}
)

type testingSuite struct {
	*base.BaseTestingSuite
}

func NewTestingSuite(ctx context.Context, testInst *e2e.TestInstallation) suite.TestingSuite {
	return &testingSuite{
		BaseTestingSuite: base.NewBaseTestingSuite(ctx, testInst, setup, testCases),
	}
}

var (
	insecureRouteManifest                        = getTestFile("insecure-route.yaml")
	secureGWPolicyManifestBackend                = getTestFile("secured-gateway-policy-with-backend.yaml")
	secureGWPolicyManifestBackendAndTlsPolicy    = getTestFile("secured-gateway-policy-with-backend-and-ref.yaml")
	secureGWPolicyManifestSvc                    = getTestFile("secured-gateway-policy-with-svc.yaml")
	secureGWPolicyManifestSvcCaCert              = getTestFile("secured-gateway-policy-with-svc-ca-cert.yaml")
	secureGWPolicyWithRbacManifest               = getTestFile("secured-gateway-policy-with-rbac.yaml")
	secureRoutePolicyManifestBackend             = getTestFile("secured-route-with-backend.yaml")
	secureRoutePolicyManifestBackendAndTlsPolicy = getTestFile("secured-route-with-backend-and-ref.yaml")
	secureRoutePolicyManifestSvc                 = getTestFile("secured-route-with-svc.yaml")
	secureRoutePolicyManifestSvcCaCert           = getTestFile("secured-route-with-svc-ca-cert.yaml")
	secureRoutePolicyWithRbacManifest            = getTestFile("secured-route-with-rbac.yaml")
)

func (s *testingSuite) TestRoutePolicyBackend() {
	s.TestInstallation.AssertionsT(s.T()).EventuallyHTTPRouteCondition(
		s.Ctx,
		"route-example-insecure",
		namespace,
		gwv1.RouteConditionAccepted,
		metav1.ConditionTrue,
	)
	// verify unprotected route works
	s.assertResponseWithoutAuth("insecureroute.com", http.StatusOK)

	s.TestInstallation.AssertionsT(s.T()).EventuallyHTTPRouteCondition(
		s.Ctx,
		"route-secure",
		namespace,
		gwv1.RouteConditionAccepted,
		metav1.ConditionTrue,
	)
	// verify a provider with a single key in jwks works
	s.assertResponse("secureroute.com", JwtOrgOne, http.StatusOK)
	s.assertResponse("secureroute.com", jwtOrgTwo, http.StatusOK)
	// verify invalid/missing tokens are caught
	s.assertResponse("secureroute.com", "nosuchkey", http.StatusUnauthorized)
	s.assertResponseWithoutAuth("secureroute.com", http.StatusUnauthorized)
}

func (s *testingSuite) TestRoutePolicyBackendAndTlsPolicy() {
	s.TestInstallation.AssertionsT(s.T()).EventuallyHTTPRouteCondition(
		s.Ctx,
		"route-secure",
		namespace,
		gwv1.RouteConditionAccepted,
		metav1.ConditionTrue,
	)
	// verify a provider with a single key in jwks works
	s.assertResponse("secureroute.com", JwtOrgOne, http.StatusOK)
	// verify invalid/missing tokens are caught
	s.assertResponse("secureroute.com", "nosuchkey", http.StatusUnauthorized)
	s.assertResponseWithoutAuth("secureroute.com", http.StatusUnauthorized)
}

func (s *testingSuite) TestRoutePolicySvcCaCert() {
	s.TestRoutePolicySvc()
}

func (s *testingSuite) TestRoutePolicySvc() {
	s.TestInstallation.AssertionsT(s.T()).EventuallyHTTPRouteCondition(
		s.Ctx,
		"route-secure",
		namespace,
		gwv1.RouteConditionAccepted,
		metav1.ConditionTrue,
	)
	// verify a provider with a single key in jwks works
	s.assertResponse("secureroute.com", JwtOrgOne, http.StatusOK)
	// verify invalid/missing tokens are caught
	s.assertResponse("secureroute.com", "nosuchkey", http.StatusUnauthorized)
	s.assertResponseWithoutAuth("secureroute.com", http.StatusUnauthorized)
}

func (s *testingSuite) TestRoutePolicyWithRbac() {
	s.TestInstallation.AssertionsT(s.T()).EventuallyHTTPRouteCondition(
		s.Ctx,
		"route-secure",
		namespace,
		gwv1.RouteConditionAccepted,
		metav1.ConditionTrue,
	)
	// verify a jwt with expected subject works
	s.assertResponse("secureroute.com", JwtOrgOne, http.StatusOK)
	// verify a jwt with unexpected subject is denied
	s.assertResponse("secureroute.com", jwtOrgFour, http.StatusForbidden)
}

func (s *testingSuite) TestGatewayPolicySvc() {
	s.TestInstallation.AssertionsT(s.T()).EventuallyHTTPRouteCondition(
		s.Ctx,
		"route-secure-gw",
		namespace,
		gwv1.RouteConditionAccepted,
		metav1.ConditionTrue,
	)
	s.assertResponse("securegateways.com", JwtOrgOne, http.StatusOK)
	// verify invalid/missing tokens are caught
	s.assertResponse("securegateways.com", "nosuchkey", http.StatusUnauthorized)
	s.assertResponseWithoutAuth("securegateways.com", http.StatusUnauthorized)
}

func (s *testingSuite) TestGatewayPolicySvcCaCert() {
	s.TestGatewayPolicySvc()
}

func (s *testingSuite) TestGatewayPolicyBackend() {
	s.TestInstallation.AssertionsT(s.T()).EventuallyHTTPRouteCondition(
		s.Ctx,
		"route-secure-gw",
		namespace,
		gwv1.RouteConditionAccepted,
		metav1.ConditionTrue,
	)
	s.assertResponse("securegateways.com", JwtOrgOne, http.StatusOK)
	s.assertResponse("securegateways.com", jwtOrgTwo, http.StatusOK)
	// verify invalid/missing tokens are caught
	s.assertResponse("securegateways.com", "nosuchkey", http.StatusUnauthorized)
	s.assertResponseWithoutAuth("securegateways.com", http.StatusUnauthorized)
}

func (s *testingSuite) TestGatewayPolicyBackendWithTlsPolicy() {
	s.TestInstallation.AssertionsT(s.T()).EventuallyHTTPRouteCondition(
		s.Ctx,
		"route-secure-gw",
		namespace,
		gwv1.RouteConditionAccepted,
		metav1.ConditionTrue,
	)
	s.assertResponse("securegateways.com", JwtOrgOne, http.StatusOK)
	// verify invalid/missing tokens are caught
	s.assertResponse("securegateways.com", "nosuchkey", http.StatusUnauthorized)
	s.assertResponseWithoutAuth("securegateways.com", http.StatusUnauthorized)
}

func (s *testingSuite) TestGatewayPolicyWithRbac() {
	s.TestInstallation.AssertionsT(s.T()).EventuallyHTTPRouteCondition(
		s.Ctx,
		"route-secure-gw",
		namespace,
		gwv1.RouteConditionAccepted,
		metav1.ConditionTrue,
	)
	// verify a jwt with expected subject works
	s.assertResponse("securegateways.com", JwtOrgOne, http.StatusOK)
	// verify a jwt with unexpected subject is denied
	s.assertResponse("securegateways.com", jwtOrgFour, http.StatusForbidden)
}

func (s *testingSuite) assertResponse(hostHeader, authHeader string, expectedStatus int) {
	gw := s.gateway()
	gw.Send(
		s.T(),
		&testmatchers.HttpResponse{
			StatusCode: expectedStatus,
		},
		curl.WithHostHeader(hostHeader),
		curl.WithHeader("Authorization", "Bearer "+authHeader),
	)
}

func (s *testingSuite) assertResponseWithoutAuth(hostHeader string, expectedStatus int) {
	gw := s.gateway()
	gw.Send(
		s.T(),
		&testmatchers.HttpResponse{
			StatusCode: expectedStatus,
		},
		curl.WithHostHeader(hostHeader),
	)
}

func (s *testingSuite) gateway() common.Gateway {
	name := types.NamespacedName{
		Namespace: namespace,
		Name:      "gateway",
	}
	return common.Gateway{
		NamespacedName: name,
		Address:        common.ResolveGatewayAddress(s.Ctx, s.TestInstallation, name),
	}
}

func getTestFile(filename string) string {
	return filepath.Join(fsutils.MustGetThisDir(), "testdata", filename)
}

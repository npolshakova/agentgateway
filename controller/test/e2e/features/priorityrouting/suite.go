//go:build e2e

package priorityrouting

import (
	"context"
	"net/http"
	"path/filepath"

	"github.com/stretchr/testify/suite"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"

	"github.com/agentgateway/agentgateway/controller/pkg/utils/fsutils"
	"github.com/agentgateway/agentgateway/controller/pkg/utils/requestutils/curl"
	"github.com/agentgateway/agentgateway/controller/test/e2e"
	"github.com/agentgateway/agentgateway/controller/test/e2e/common"
	"github.com/agentgateway/agentgateway/controller/test/e2e/tests/base"
	"github.com/agentgateway/agentgateway/controller/test/gomega/matchers"
)

var (
	manifest = filepath.Join(fsutils.MustGetThisDir(), "testdata", "priority-routing.yaml")

	testCases = map[string]*base.TestCase{
		"TestEndpointGroupPriorityRouting": {
			Manifests: []string{manifest},
		},
	}
)

const namespace = "agentgateway-base"

type testingSuite struct {
	*base.BaseTestingSuite
}

func NewTestingSuite(ctx context.Context, testInst *e2e.TestInstallation) suite.TestingSuite {
	return &testingSuite{
		BaseTestingSuite: base.NewBaseTestingSuite(ctx, testInst, base.TestCase{}, testCases),
	}
}

func (s *testingSuite) TestEndpointGroupPriorityRouting() {
	// Wait for the backend to be accepted.
	s.TestInstallation.AssertionsT(s.T()).EventuallyAgwBackendCondition(
		s.Ctx, "priority-backend", namespace, "Accepted", metav1.ConditionTrue,
	)

	// The first endpoint (httpbin) has the highest priority and should receive traffic.
	// Verify the request is routed through the endpointGroup to httpbin.
	common.BaseGateway.Send(
		s.T(),
		&matchers.HttpResponse{
			StatusCode: http.StatusOK,
		},
		curl.WithHostHeader("priority.example.com"),
		curl.WithPath("/status/200"),
	)
}

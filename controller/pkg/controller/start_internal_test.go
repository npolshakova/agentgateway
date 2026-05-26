package controller

import (
	"testing"

	"istio.io/istio/pkg/test/util/assert"
	"k8s.io/apimachinery/pkg/api/meta"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/utils/ptr"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/agentgateway/agentgateway/controller/pkg/reports"
)

func TestNormalizeProxyImageTag(t *testing.T) {
	tests := []struct {
		name string
		tag  string
		want string
	}{
		{
			name: "adds v prefix",
			tag:  "1.2.3",
			want: "v1.2.3",
		},
		{
			name: "preserves v-prefixed tag",
			tag:  "v1.2.3",
			want: "v1.2.3",
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got := normalizeProxyImageTag(tt.tag)
			if got != tt.want {
				t.Fatalf("expected tag %q, got %q", tt.want, got)
			}
		})
	}
}

func TestBuildGatewayStatusAfterSuccessfulDeploy(t *testing.T) {
	desiredAddresses := []gwv1.GatewayStatusAddress{{
		Type:  ptr.To(gwv1.IPAddressType),
		Value: "10.0.0.1",
	}}

	t.Run("clears controller-owned resource apply error", func(t *testing.T) {
		gw := gatewayForStatusTest()
		gw.Generation = 3
		gw.Status.Conditions = append(gw.Status.Conditions, metav1.Condition{
			Type:               string(gwv1.GatewayConditionProgrammed),
			Status:             metav1.ConditionFalse,
			ObservedGeneration: 2,
			Reason:             reports.GatewayResourceErrorReason,
			Message:            "failed to apply object apps/v1, Kind=Deployment default/test: field is immutable",
		})

		status, needsUpdate := buildGatewayStatusAfterSuccessfulDeploy(gw, desiredAddresses)

		assert.Equal(t, true, needsUpdate)
		programmed := meta.FindStatusCondition(status.Conditions, string(gwv1.GatewayConditionProgrammed))
		assert.Equal(t, true, programmed != nil)
		assert.Equal(t, metav1.ConditionTrue, programmed.Status)
		assert.Equal(t, string(gwv1.GatewayReasonProgrammed), programmed.Reason)
		assert.Equal(t, reports.GatewayProgrammedMessage, programmed.Message)
		assert.Equal(t, gw.Generation, programmed.ObservedGeneration)
		assert.Equal(t, desiredAddresses, status.Addresses)
	})

	t.Run("preserves non-controller programmed failures", func(t *testing.T) {
		gw := gatewayForStatusTest()
		gw.Generation = 4
		gw.Status.Conditions = append(gw.Status.Conditions, metav1.Condition{
			Type:               string(gwv1.GatewayConditionProgrammed),
			Status:             metav1.ConditionFalse,
			ObservedGeneration: 4,
			Reason:             string(gwv1.GatewayReasonAddressNotUsable),
			Message:            "Hostname addresses may not be used",
		})

		status, needsUpdate := buildGatewayStatusAfterSuccessfulDeploy(gw, desiredAddresses)

		assert.Equal(t, true, needsUpdate)
		programmed := meta.FindStatusCondition(status.Conditions, string(gwv1.GatewayConditionProgrammed))
		assert.Equal(t, true, programmed != nil)
		assert.Equal(t, metav1.ConditionFalse, programmed.Status)
		assert.Equal(t, string(gwv1.GatewayReasonAddressNotUsable), programmed.Reason)
		assert.Equal(t, desiredAddresses, status.Addresses)
	})

	t.Run("skips update when nothing changes", func(t *testing.T) {
		gw := gatewayForStatusTest()
		gw.Status.Addresses = desiredAddresses
		gw.Status.Conditions = append(gw.Status.Conditions, metav1.Condition{
			Type:               string(gwv1.GatewayConditionProgrammed),
			Status:             metav1.ConditionTrue,
			ObservedGeneration: gw.Generation,
			Reason:             string(gwv1.GatewayReasonProgrammed),
			Message:            reports.GatewayProgrammedMessage,
		})

		status, needsUpdate := buildGatewayStatusAfterSuccessfulDeploy(gw, desiredAddresses)

		assert.Equal(t, false, needsUpdate)
		assert.Equal(t, gw.Status, status)
	})
}

func gatewayForStatusTest() *gwv1.Gateway {
	return &gwv1.Gateway{
		ObjectMeta: metav1.ObjectMeta{
			Namespace: "default",
			Name:      "test-gateway",
		},
	}
}

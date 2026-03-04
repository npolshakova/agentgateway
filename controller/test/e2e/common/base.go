//go:build e2e

package common

import (
	"context"
	"fmt"
	"log"
	"net"
	"net/http"
	"os"
	"strconv"
	"strings"
	"sync"
	"testing"
	"time"

	"istio.io/istio/pkg/test/util/assert"
	"istio.io/istio/pkg/test/util/retry"
	corev1 "k8s.io/api/core/v1"
	"k8s.io/apimachinery/pkg/types"

	"github.com/agentgateway/agentgateway/controller/pkg/utils/kubeutils/portforward"
	"github.com/agentgateway/agentgateway/controller/pkg/utils/requestutils/curl"
	"github.com/agentgateway/agentgateway/controller/test/e2e"
	"github.com/agentgateway/agentgateway/controller/test/gomega/matchers"
)

func SetupBaseConfig(ctx context.Context, t *testing.T, installation *e2e.TestInstallation, manifests ...string) {
	err := installation.ClusterContext.IstioClient.ApplyYAMLFiles("", manifests...)
	assert.NoError(t, err)
}

func SetupBaseGateway(ctx context.Context, installation *e2e.TestInstallation, name types.NamespacedName) {
	baseInstallation = installation
	BaseGateway = Gateway{
		NamespacedName: name,
		Address:        ResolveGatewayAddress(ctx, installation, name),
	}
}

var (
	gatewayAddressMu sync.Mutex
	gatewayAddresses = map[types.NamespacedName]string{}
	baseInstallation *e2e.TestInstallation
)

// ResolveGatewayAddress returns a reachable gateway address for e2e traffic.
// If USE_PORTFORWARD is set, tests use a local port-forward; otherwise, they use the LoadBalancer address.
func ResolveGatewayAddress(ctx context.Context, installation *e2e.TestInstallation, name types.NamespacedName) string {
	if !shouldUsePortForward() {
		return installation.Assertions.EventuallyGatewayAddress(ctx, name.Name, name.Namespace)
	}

	gatewayAddressMu.Lock()
	defer gatewayAddressMu.Unlock()
	if addr, ok := gatewayAddresses[name]; ok {
		return addr
	}

	addr, err := setupGatewayPortForwards(ctx, installation, name)
	if err != nil {
		log.Printf(
			"WARN: USE_PORTFORWARD is set but port-forward setup failed for Gateway %s/%s: %v; falling back to LoadBalancer address",
			name.Namespace,
			name.Name,
			err,
		)
		// Do not cache the fallback LB address. Keep retrying port-forward resolution on subsequent calls.
		return installation.Assertions.EventuallyGatewayAddress(ctx, name.Name, name.Namespace)
	}
	gatewayAddresses[name] = addr
	return addr
}

func shouldUsePortForward() bool {
	_, set := os.LookupEnv("USE_PORTFORWARD")
	return set
}

func setupGatewayPortForwards(ctx context.Context, installation *e2e.TestInstallation, name types.NamespacedName) (string, error) {
	svc := &corev1.Service{}
	if err := installation.ClusterContext.Client.Get(ctx, name, svc); err != nil {
		return "", fmt.Errorf("failed to get gateway service %s/%s: %w", name.Namespace, name.Name, err)
	}
	if len(svc.Spec.Ports) == 0 {
		return "", fmt.Errorf("gateway service %s/%s has no ports", name.Namespace, name.Name)
	}

	forwarders := make([]portforward.PortForwarder, 0, len(svc.Spec.Ports))
	defaultAddress := ""
	for _, port := range svc.Spec.Ports {
		remotePort := int(port.Port)
		options := []portforward.Option{
			portforward.WithService(name.Name, name.Namespace),
		}
		// Privileged ports like 80 cannot be bound locally without elevation.
		if remotePort < 1024 {
			options = append(options, portforward.WithRemotePort(remotePort))
		} else {
			options = append(options, portforward.WithPorts(remotePort, remotePort))
		}

		forwarder, err := installation.Actions.Kubectl().StartPortForward(ctx, options...)
		if err != nil {
			for _, started := range forwarders {
				started.Close()
			}
			return "", fmt.Errorf("failed to port-forward service %s/%s on port %d: %w", name.Namespace, name.Name, remotePort, err)
		}
		forwarders = append(forwarders, forwarder)

		if defaultAddress == "" || port.Port == 80 || strings.EqualFold(port.Name, "http") {
			defaultAddress = forwarder.Address()
		}
	}

	go func() {
		<-ctx.Done()
		for _, forwarder := range forwarders {
			forwarder.Close()
		}
	}()

	return defaultAddress, nil
}

type Gateway struct {
	types.NamespacedName
	Address string
}

var BaseGateway Gateway

func (g *Gateway) Send(t *testing.T, match *matchers.HttpResponse, opts ...curl.Option) {
	resp := g.SendWithResponse(t, match, opts...)
	_ = resp.Body.Close()
}

func (g *Gateway) SendWithResponse(t *testing.T, match *matchers.HttpResponse, opts ...curl.Option) http.Response {
	address := g.ResolvedAddress()
	fullOpts := append(GatewayAddressOptions(address), opts...)
	var passedRes http.Response
	retry.UntilSuccessOrFail(t, func() error {
		r, err := curl.ExecuteRequest(fullOpts...)
		if err != nil {
			return err
		}
		mm := matchers.HaveHttpResponse(match)
		success, err := mm.Match(r)
		if err != nil {
			r.Body.Close()
			return err
		}
		if !success {
			r.Body.Close()
			return fmt.Errorf("match failed: %v", mm.FailureMessage(r))
		}
		passedRes = *r
		return nil
	}, retry.Timeout(time.Second*300))
	return passedRes
}

func (g *Gateway) ResolvedAddress() string {
	address := g.Address
	if shouldUsePortForward() && g.NamespacedName.Name != "" && !addressHasPort(address) && baseInstallation != nil {
		return ResolveGatewayAddress(context.Background(), baseInstallation, g.NamespacedName)
	}
	return address
}

func GatewayAddressOptions(address string) []curl.Option {
	host, port, err := net.SplitHostPort(address)
	if err != nil {
		return []curl.Option{curl.WithHost(address)}
	}
	if strings.EqualFold(host, "localhost") {
		host = "127.0.0.1"
	}
	parsedPort, err := strconv.Atoi(port)
	if err != nil {
		return []curl.Option{curl.WithHost(address)}
	}
	return []curl.Option{curl.WithHost(host), curl.WithPort(parsedPort)}
}

func addressHasPort(address string) bool {
	_, _, err := net.SplitHostPort(address)
	return err == nil
}

package temporalhost

import (
	"crypto/tls"
	"crypto/x509"
	"testing"
)

func TestTLSProviderCannotDowngradeDynamicListenerOrClientVerification(t *testing.T) {
	base := &tls.Config{MinVersion: tls.VersionTLS12, ClientAuth: tls.RequireAndVerifyClientCert, ClientCAs: x509.NewCertPool()}
	selected := base.Clone()
	base.GetConfigForClient = func(*tls.ClientHelloInfo) (*tls.Config, error) { return selected, nil }
	secured, err := secure(base, true)
	if err != nil || secured.MinVersion != tls.VersionTLS13 || base.MinVersion != tls.VersionTLS12 {
		t.Fatal("listener minimum was not independently constrained", err)
	}
	dynamic, err := secured.GetConfigForClient(&tls.ClientHelloInfo{})
	if err != nil || dynamic.MinVersion != tls.VersionTLS13 {
		t.Fatal("dynamic selection bypassed TLS minimum", err)
	}
	selected.ClientAuth = tls.NoClientCert
	if _, err := secured.GetConfigForClient(&tls.ClientHelloInfo{}); err == nil {
		t.Fatal("dynamic selection disabled client authentication")
	}
	if _, err := secure(&tls.Config{InsecureSkipVerify: true}, false); err == nil {
		t.Fatal("client disabled peer verification")
	}
	if _, err := secure(nil, true); err == nil {
		t.Fatal("missing TLS became plaintext")
	}
}

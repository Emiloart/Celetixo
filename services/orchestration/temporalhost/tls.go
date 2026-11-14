package temporalhost

import (
	"crypto/tls"

	"go.temporal.io/server/common/rpc/encryption"
)

// Require TLS 1.3 on each provider result, including dynamically selected server
// configuration. A callback must not undo the top-level listener requirements.
type strictTLS struct{ encryption.TLSConfigProvider }

func secure(configuration *tls.Config, server bool) (*tls.Config, error) {
	if configuration == nil || configuration.InsecureSkipVerify {
		return nil, ErrConfiguration
	}
	if server {
		if configuration.ClientAuth != tls.RequireAndVerifyClientCert || configuration.ClientCAs == nil {
			return nil, ErrConfiguration
		}
	} else if configuration.RootCAs == nil || configuration.ServerName == "" {
		return nil, ErrConfiguration
	}
	result := configuration.Clone()
	result.MinVersion = tls.VersionTLS13
	if original := configuration.GetConfigForClient; original != nil {
		result.GetConfigForClient = func(hello *tls.ClientHelloInfo) (*tls.Config, error) {
			selected, err := original(hello)
			if err != nil {
				return nil, err
			}
			if selected == nil {
				return nil, nil // standard TLS semantics: retain this secured configuration
			}
			return secure(selected, true)
		}
	}
	return result, nil
}

func (p strictTLS) GetInternodeServerConfig() (*tls.Config, error) {
	c, err := p.TLSConfigProvider.GetInternodeServerConfig()
	if err != nil {
		return nil, err
	}
	return secure(c, true)
}
func (p strictTLS) GetFrontendServerConfig() (*tls.Config, error) {
	c, err := p.TLSConfigProvider.GetFrontendServerConfig()
	if err != nil {
		return nil, err
	}
	return secure(c, true)
}
func (p strictTLS) GetInternodeClientConfig() (*tls.Config, error) {
	c, err := p.TLSConfigProvider.GetInternodeClientConfig()
	if err != nil {
		return nil, err
	}
	return secure(c, false)
}
func (p strictTLS) GetFrontendClientConfig() (*tls.Config, error) {
	c, err := p.TLSConfigProvider.GetFrontendClientConfig()
	if err != nil {
		return nil, err
	}
	return secure(c, false)
}
func (p strictTLS) GetRemoteClusterClientConfig(host string) (*tls.Config, error) {
	c, err := p.TLSConfigProvider.GetRemoteClusterClientConfig(host)
	if err != nil {
		return nil, err
	}
	return secure(c, false)
}

package temporalhost

import (
	"bytes"
	"encoding/json"
	"errors"
	"io"
	"net"
	"path/filepath"
	"strconv"
	"strings"

	"github.com/Emiloart/Celetixo/services/orchestration/operatorfile"
	"github.com/Emiloart/Celetixo/services/orchestration/temporalauth"
	"go.temporal.io/server/common/config"
	"go.temporal.io/server/common/membership/static"
	"go.temporal.io/server/common/primitives"
	"gopkg.in/yaml.v3"
)

var ErrConfiguration = errors.New("invalid private Temporal host configuration")

type Failure struct {
	Stage string
	cause error
}

func (e *Failure) Error() string              { return "Temporal host " + e.Stage + " failed" }
func (e *Failure) Unwrap() error              { return e.cause }
func failure(stage string, cause error) error { return &Failure{Stage: stage, cause: cause} }

type operatorConfiguration struct {
	ContractVersion       uint32                       `json:"contract_version"`
	ServerFile            string                       `json:"server_file"`
	IdentityFile          string                       `json:"identity_file"`
	DatabasePasswordFiles map[string]string            `json:"database_password_files"`
	StaticHosts           map[string]hostConfiguration `json:"static_hosts"`
}
type hostConfiguration struct {
	Self string   `json:"self"`
	All  []string `json:"all"`
}
type identityConfiguration struct {
	ContractVersion uint32                 `json:"contract_version"`
	Bindings        []temporalauth.Binding `json:"bindings"`
}

type Configuration struct {
	server      *config.Config
	policy      *temporalauth.Policy
	staticHosts map[primitives.ServiceName]static.Hosts
}

func decodeJSON(path string, target any) error {
	data, err := operatorfile.Read(path)
	if err != nil {
		return ErrConfiguration
	}
	decoder := json.NewDecoder(bytes.NewReader(data))
	decoder.DisallowUnknownFields()
	if err := decoder.Decode(target); err != nil {
		return failure("operator JSON decoding", err)
	}
	if err := decoder.Decode(new(any)); err != io.EOF {
		return ErrConfiguration
	}
	return nil
}

func Load(path string, serving bool) (*Configuration, error) {
	var operator operatorConfiguration
	if err := decodeJSON(path, &operator); err != nil || operator.ContractVersion != 1 {
		return nil, failure("operator configuration", err)
	}
	data, err := operatorfile.Read(operator.ServerFile)
	if err != nil {
		return nil, failure("server configuration file", err)
	}
	// Decode data only. Do not run the vendor loader's optional environment-based
	// templating or accept unknown YAML fields that could hide an operator typo.
	server := &config.Config{}
	decoder := yaml.NewDecoder(bytes.NewReader(data))
	decoder.KnownFields(true)
	if err := decoder.Decode(server); err != nil {
		return nil, failure("server YAML decoding", err)
	}
	if err := decoder.Decode(new(any)); err != io.EOF {
		return nil, ErrConfiguration
	}
	if err := configureDatabases(server, operator.DatabasePasswordFiles); err != nil {
		return nil, failure("database configuration", err)
	}
	selected := &Configuration{server: server}
	if serving {
		var identities identityConfiguration
		if err := decodeJSON(operator.IdentityFile, &identities); err != nil || identities.ContractVersion != 1 {
			return nil, ErrConfiguration
		}
		selected.policy, err = temporalauth.New(identities.Bindings, nil)
		if err != nil || validateServing(server) != nil {
			return nil, ErrConfiguration
		}
		selected.staticHosts, err = validateHosts(server, operator.StaticHosts)
		if err != nil {
			return nil, err
		}
	}
	return selected, nil
}

func configureDatabases(server *config.Config, passwords map[string]string) error {
	p := &server.Persistence
	if p.DefaultStore == "" || p.VisibilityStore == "" || p.DefaultStore == p.VisibilityStore ||
		p.SecondaryVisibilityStore != "" || len(p.DataStores) != 2 || len(passwords) != 2 {
		return ErrConfiguration
	}
	databases := map[string]bool{}
	for _, name := range []string{p.DefaultStore, p.VisibilityStore} {
		store, ok := p.DataStores[name]
		if !ok || store.SQL == nil || store.Cassandra != nil || store.Elasticsearch != nil || store.CustomDataStoreConfig != nil || store.FaultInjection != nil {
			return ErrConfiguration
		}
		sql := store.SQL
		if sql.PluginName != "postgres12_pgx" || sql.Connect != nil || sql.Password != "" || sql.PasswordCommand != nil ||
			!databaseIdentity(sql.User) || !databaseIdentity(sql.DatabaseName) || sql.MaxConns < 1 || sql.MaxConns > 8 || sql.MaxIdleConns < 0 || sql.MaxIdleConns > sql.MaxConns {
			return ErrConfiguration
		}
		if databases[sql.DatabaseName] {
			return ErrConfiguration
		}
		databases[sql.DatabaseName] = true
		// This reviewed local profile uses SCRAM Unix sockets. Network PostgreSQL
		// needs a separate explicit TLS policy, not a plaintext fallback.
		host := sql.ConnectAttributes["host"]
		if !filepath.IsAbs(host) || sql.ConnectAttributes["sslmode"] != "disable" ||
			sql.ConnectAttributes["require_auth"] != "scram-sha-256" || (sql.TLS != nil && sql.TLS.Enabled) ||
			sql.ConnectAddr != "localhost:5432" || sql.ConnectAttributes["port"] != "5432" {
			return ErrConfiguration
		}
		allowed := map[string]bool{"host": true, "port": true, "sslmode": true, "require_auth": true,
			"statement_timeout": true, "lock_timeout": true, "connect_timeout": true}
		for key := range sql.ConnectAttributes {
			if !allowed[key] {
				return ErrConfiguration
			}
		}
		for _, key := range []string{"statement_timeout", "lock_timeout"} {
			value, err := strconv.ParseUint(sql.ConnectAttributes[key], 10, 32)
			if err != nil || value == 0 || value > 30000 {
				return ErrConfiguration
			}
		}
		if sql.ConnectAttributes["connect_timeout"] != "3" {
			return ErrConfiguration
		}
		secret, err := operatorfile.Read(passwords[name])
		if err != nil {
			return ErrConfiguration
		}
		value := strings.TrimSuffix(string(secret), "\n")
		if value == "" || len(value) > 4096 || strings.ContainsAny(value, "\r\n\x00") {
			return ErrConfiguration
		}
		sql.Password = value
	}
	return nil
}

func databaseIdentity(value string) bool {
	if value == "" || len(value) > 63 {
		return false
	}
	for _, ch := range value {
		if !(ch >= 'a' && ch <= 'z' || ch >= '0' && ch <= '9' || ch == '_') {
			return false
		}
	}
	return true
}

func privateFile(path string) bool { _, err := operatorfile.Read(path); return err == nil }

func validateClient(client config.ClientTLS) bool {
	if client.DisableHostVerification || client.ServerName == "" || len(client.RootCAFiles) == 0 || len(client.RootCAData) != 0 {
		return false
	}
	for _, path := range client.RootCAFiles {
		if !privateFile(path) {
			return false
		}
	}
	return true
}

func validateServing(server *config.Config) error {
	if server.Global.PProf.Port != 0 || len(server.Global.TLS.RemoteClusters) != 0 || len(server.Services) != 4 ||
		server.Log.Development || (server.Log.Level != "warn" && server.Log.Level != "error") || server.Log.OutputFile != "" || server.PublicClient.HTTPHostPort != "" {
		return ErrConfiguration
	}
	for _, name := range []string{"frontend", "history", "matching", "worker"} {
		service, ok := server.Services[name]
		if !ok || service.RPC.HTTPPort != 0 {
			return ErrConfiguration
		}
	}
	for _, group := range []config.GroupTLS{server.Global.TLS.Frontend, server.Global.TLS.Internode} {
		if !group.Server.RequireClientAuth || group.Server.CertData != "" || group.Server.KeyData != "" ||
			len(group.Server.ClientCAData) != 0 || len(group.Server.ClientCAFiles) == 0 || len(group.PerHostOverrides) != 0 ||
			!privateFile(group.Server.CertFile) || !privateFile(group.Server.KeyFile) || !validateClient(group.Client) {
			return ErrConfiguration
		}
		for _, path := range group.Server.ClientCAFiles {
			if !privateFile(path) {
				return ErrConfiguration
			}
		}
	}
	w := server.Global.TLS.SystemWorker
	if w.CertData != "" || w.KeyData != "" || !privateFile(w.CertFile) || !privateFile(w.KeyFile) || !validateClient(w.Client) {
		return ErrConfiguration
	}
	if server.DynamicConfigClient == nil || !privateFile(server.DynamicConfigClient.Filepath) {
		return ErrConfiguration
	}
	if err := server.Validate(); err != nil {
		return ErrConfiguration
	}
	return nil
}

func privateAddress(value string) bool {
	host, port, err := net.SplitHostPort(value)
	if err != nil {
		return false
	}
	ip := net.ParseIP(host)
	number, err := strconv.ParseUint(port, 10, 16)
	return err == nil && number > 0 && ip != nil && (ip.IsPrivate() || ip.IsLoopback())
}

// Static peer discovery avoids an unauthenticated membership listener. Every
// selected RPC peer still uses the independent internode TLS trust boundary.
func validateHosts(server *config.Config, input map[string]hostConfiguration) (map[primitives.ServiceName]static.Hosts, error) {
	if len(input) != 4 {
		return nil, ErrConfiguration
	}
	result := make(map[primitives.ServiceName]static.Hosts, 4)
	for _, name := range []string{"frontend", "history", "matching", "worker"} {
		selected, ok := input[name]
		if !ok || len(selected.All) == 0 || len(selected.All) > 16 || !privateAddress(selected.Self) {
			return nil, ErrConfiguration
		}
		rpc := server.Services[name].RPC
		host := rpc.BindOnIP
		if rpc.BindOnLocalHost && host == "" {
			host = "127.0.0.1"
		}
		if selected.Self != net.JoinHostPort(host, strconv.Itoa(rpc.GRPCPort)) {
			return nil, ErrConfiguration
		}
		seen := map[string]bool{}
		for _, address := range selected.All {
			if !privateAddress(address) || seen[address] {
				return nil, ErrConfiguration
			}
			seen[address] = true
		}
		if !seen[selected.Self] {
			return nil, ErrConfiguration
		}
		result[primitives.ServiceName(name)] = static.Hosts{Self: selected.Self, All: append([]string(nil), selected.All...)}
	}
	return result, nil
}

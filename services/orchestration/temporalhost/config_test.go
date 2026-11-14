package temporalhost

import (
	"encoding/json"
	"os"
	"path/filepath"
	"testing"

	"go.temporal.io/server/common/config"
	"gopkg.in/yaml.v3"
)

func TestOperatorLoaderBindsSeparateDatabasesAndPrivateSecrets(t *testing.T) {
	root := t.TempDir()
	write := func(name string, value []byte) string {
		t.Helper()
		path := filepath.Join(root, name)
		if err := os.WriteFile(path, value, 0600); err != nil {
			t.Fatal(err)
		}
		return path
	}
	makeSQL := func(database string) *config.SQL {
		return &config.SQL{PluginName: "postgres12_pgx", User: "runtime", DatabaseName: database,
			ConnectAddr: "localhost:5432", ConnectProtocol: "unix", MaxConns: 2,
			ConnectAttributes: map[string]string{"host": root, "port": "5432", "sslmode": "disable", "require_auth": "scram-sha-256",
				"connect_timeout": "3", "statement_timeout": "5000", "lock_timeout": "5000"}}
	}
	server := config.Config{Persistence: config.Persistence{DefaultStore: "workflow", VisibilityStore: "visibility",
		DataStores: map[string]config.DataStore{"workflow": {SQL: makeSQL("workflow")}, "visibility": {SQL: makeSQL("visibility")}}}}
	serverPath := filepath.Join(root, "server.yaml")
	storeServer := func() {
		t.Helper()
		data, err := yaml.Marshal(server)
		if err != nil {
			t.Fatal(err)
		}
		write("server.yaml", data)
	}
	storeServer()
	operator := operatorConfiguration{ContractVersion: 1, ServerFile: serverPath, DatabasePasswordFiles: map[string]string{
		"workflow":   write("workflow.secret", []byte(" synthetic value with spaces \n")),
		"visibility": write("visibility.secret", []byte("synthetic fixture value\n")),
	}}
	data, err := json.Marshal(operator)
	if err != nil {
		t.Fatal(err)
	}
	path := write("operator.json", data)
	loaded, err := Load(path, false)
	if err != nil {
		t.Fatal(err)
	}
	if loaded.server.Persistence.DataStores["workflow"].SQL.Password != " synthetic value with spaces " {
		t.Fatal("credential bytes were silently normalized")
	}
	server.Persistence.DataStores["visibility"].SQL.DatabaseName = "workflow"
	storeServer()
	if _, err := Load(path, false); err == nil {
		t.Fatal("workflow and visibility schema ledgers share a database")
	}
	server.Persistence.DataStores["visibility"].SQL.DatabaseName = "visibility"
	server.Persistence.DataStores["workflow"].SQL.PasswordCommand = &config.PasswordCommandConfig{Command: "/untrusted/not-executed"}
	storeServer()
	if _, err := Load(path, false); err == nil {
		t.Fatal("configuration executed or accepted an external credential command")
	}
}

func TestOperatorLoaderRejectsUnknownJSONFields(t *testing.T) {
	path := filepath.Join(t.TempDir(), "operator.json")
	if err := os.WriteFile(path, []byte(`{"contract_version":1,"unsupported_authority":true}`), 0600); err != nil {
		t.Fatal(err)
	}
	if _, err := Load(path, false); err == nil {
		t.Fatal("unknown operator semantics admitted")
	}
}

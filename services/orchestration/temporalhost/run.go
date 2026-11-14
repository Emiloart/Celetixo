package temporalhost

import (
	"go.temporal.io/server/common/authorization"
	"go.temporal.io/server/common/config"
	"go.temporal.io/server/common/log"
	"go.temporal.io/server/common/metrics"
	_ "go.temporal.io/server/common/persistence/sql/sqlplugin/postgresql"
	"go.temporal.io/server/common/rpc/encryption"
	"go.temporal.io/server/temporal"
	"go.temporal.io/server/tools/common/schema"
	sqltool "go.temporal.io/server/tools/sql"
)

func (c *Configuration) Serve() error {
	if c.policy == nil {
		return ErrConfiguration
	}
	logger := log.NewZapLogger(log.BuildZapLogger(c.server.Log))
	for _, store := range c.server.Persistence.DataStores {
		if err := checkRole(store.SQL, logger, false); err != nil {
			return failure("runtime database privilege check", err)
		}
	}
	provider, err := encryption.NewTLSConfigProviderFromConfig(c.server.Global.TLS, metrics.NoopMetricsHandler, logger, nil)
	if err != nil {
		return failure("TLS configuration", err)
	}
	server, err := temporal.NewServer(
		temporal.WithConfig(c.server), temporal.WithLogger(logger),
		temporal.ForServices([]string{"frontend", "history", "matching", "worker"}),
		temporal.WithStaticHosts(c.staticHosts),
		temporal.WithTLSConfigFactory(strictTLS{provider}),
		temporal.WithAuthorizer(c.policy),
		temporal.WithClaimMapper(func(*config.Config) authorization.ClaimMapper { return c.policy }),
		temporal.InterruptOn(temporal.InterruptCh()),
	)
	if err != nil {
		return failure("service construction", err)
	}
	if err := server.Start(); err != nil {
		return failure("service startup", err)
	}
	return nil
}

// Migrate is an explicit operator operation, using the pinned server's embedded
// schemas and migration implementation. Serving never runs DDL. Empty/partially
// initialized state is distinguished before setup; existing ledgers are not reset.
func (c *Configuration) Migrate() error {
	logger := log.NewZapLogger(log.BuildZapLogger(log.Config{Level: "warn", Format: "json"}))
	for _, store := range []struct{ name, schema string }{
		{c.server.Persistence.DefaultStore, "postgresql/v12/temporal"},
		{c.server.Persistence.VisibilityStore, "postgresql/v12/visibility"},
	} {
		if err := checkRole(c.server.Persistence.DataStores[store.name].SQL, logger, true); err != nil {
			return failure("migration database privilege check", err)
		}
		connection, err := sqltool.NewConnection(c.server.Persistence.DataStores[store.name].SQL, logger)
		if err != nil {
			return failure("migration database connection", err)
		}
		err = func() error {
			defer connection.Close()
			tables, err := connection.ListTables()
			if err != nil {
				return err
			}
			if len(tables) == 0 {
				if err := schema.NewSetupSchemaTask(connection, &schema.SetupConfig{InitialVersion: "0.0"}, logger).Run(); err != nil {
					return err
				}
			} else if _, err := connection.ReadSchemaVersion(); err != nil {
				return err // retain partial state for operator review, never invent an empty ledger
			}
			return schema.NewUpdateSchemaTask(connection, &schema.UpdateConfig{DBName: c.server.Persistence.DataStores[store.name].SQL.DatabaseName, SchemaName: store.schema}, logger).Run()
		}()
		if err != nil {
			return failure("vendor schema migration", err)
		}
	}
	return nil
}

func checkRole(configuration *config.SQL, logger log.Logger, migration bool) error {
	connection, err := sqltool.NewConnection(configuration, logger)
	if err != nil {
		return err
	}
	defer connection.Close()
	query := `DO $$ BEGIN
IF current_user<>session_user OR EXISTS (SELECT 1 FROM pg_roles WHERE rolname=session_user AND (rolsuper OR rolcreatedb OR rolcreaterole OR rolreplication OR rolbypassrls)) THEN
 RAISE EXCEPTION 'Temporal database role exceeds its privileges';
END IF;`
	if !migration {
		query += `
IF has_database_privilege(session_user,current_database(),'CREATE') OR has_schema_privilege(session_user,'public','CREATE') OR
EXISTS (SELECT 1 FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='public' AND c.relkind IN ('r','p') AND pg_has_role(session_user,c.relowner,'MEMBER')) THEN
 RAISE EXCEPTION 'Temporal runtime role owns schema or tables';
END IF;`
	}
	return connection.Exec(query + " END; $$")
}

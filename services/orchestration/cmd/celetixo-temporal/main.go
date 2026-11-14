package main

import (
	"fmt"
	"os"
	"runtime"

	"github.com/Emiloart/Celetixo/services/orchestration/temporalhost"
)

func main() {
	if err := execute(); err != nil {
		// Dependency diagnostics may contain connection material. Only a fixed
		// stage error is returned to the process boundary.
		fmt.Fprintln(os.Stderr, "celetixo-temporal:", err)
		os.Exit(1)
	}
}

func execute() error {
	if runtime.GOOS != "linux" || os.Geteuid() == 0 || len(os.Args) != 4 || os.Args[2] != "--config" || (os.Args[1] != "serve" && os.Args[1] != "migrate") {
		return temporalhost.ErrConfiguration
	}
	serving := os.Args[1] == "serve"
	configuration, err := temporalhost.Load(os.Args[3], serving)
	if err != nil {
		return err
	}
	if serving {
		return configuration.Serve()
	}
	return configuration.Migrate()
}

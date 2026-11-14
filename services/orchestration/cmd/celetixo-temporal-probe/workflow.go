package main

// This synthetic workflow verifies the real SDK/host coordination boundary. Its
// only activity hashes bytes; it never mutates Engine state. Domain workflows
// must separately reconcile actual Engine effects and exact receipts.
import (
	"context"
	"crypto/sha256"
	"crypto/tls"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"os/signal"
	"syscall"
	"time"

	platform "github.com/Emiloart/Celetixo/services/orchestration/gen/platformv1"
	"github.com/Emiloart/Celetixo/services/orchestration/operatorfile"
	"github.com/Emiloart/Celetixo/services/orchestration/protocol"
	enums "go.temporal.io/api/enums/v1"
	"go.temporal.io/api/serviceerror"
	"go.temporal.io/sdk/activity"
	"go.temporal.io/sdk/client"
	"go.temporal.io/sdk/temporal"
	"go.temporal.io/sdk/worker"
	"go.temporal.io/sdk/workflow"
	"google.golang.org/grpc"
	"google.golang.org/grpc/status"
	"google.golang.org/protobuf/proto"
)

const probeType = "celetixo.fixture.coordination.v1"
const probeActivityType = "celetixo.fixture.digest.v1"

// Default SDK logging must not render payloads or connection details here.
type quietLogger struct{}

func (quietLogger) Debug(string, ...interface{}) {}
func (quietLogger) Info(string, ...interface{})  {}
func (quietLogger) Warn(string, ...interface{})  {}
func (quietLogger) Error(string, ...interface{}) {}

func probeWorkflow(ctx workflow.Context, canonical []byte) (string, error) {
	workflow.GetVersion(ctx, "coordination-probe", workflow.DefaultVersion, 1)
	if err := workflow.SetQueryHandler(ctx, "checkpoint", func() (bool, error) { return true, nil }); err != nil {
		return "", err
	}
	var complete bool
	workflow.GetSignalChannel(ctx, "finish").Receive(ctx, &complete)
	if !complete {
		return "", temporal.NewNonRetryableApplicationError("invalid probe signal", "ProbeInput", nil)
	}
	ctx = workflow.WithActivityOptions(ctx, workflow.ActivityOptions{
		StartToCloseTimeout: 5 * time.Second, ScheduleToCloseTimeout: 20 * time.Second,
		RetryPolicy: &temporal.RetryPolicy{InitialInterval: time.Second, MaximumInterval: 2 * time.Second, MaximumAttempts: 3},
	})
	var result string
	if err := workflow.ExecuteActivity(ctx, probeActivityType, canonical).Get(ctx, &result); err != nil {
		return "", err
	}
	return result, nil
}

func probeActivity(_ context.Context, canonical []byte) (string, error) {
	var envelope platform.TaskIntentEnvelope
	if protocol.Decode(canonical, &envelope) != nil {
		return "", temporal.NewNonRetryableApplicationError("invalid canonical shape", "ProbeInput", nil)
	}
	reencoded, err := (proto.MarshalOptions{Deterministic: true}).Marshal(&envelope)
	if err != nil {
		return "", err
	}
	hash := sha256.Sum256(reencoded)
	return hex.EncodeToString(hash[:]), nil
}

func coordinationProbe(mode string, selected *configuration, namespace string, transport *tls.Config) error {
	if selected.ProbeWorkflowID == "" && mode != "run-worker" {
		return errors.New("explicit probe workflow identity")
	}
	if mode != "run-worker" && mode != "start-probe" && selected.ProbeRunID == "" {
		return errors.New("explicit probe run identity")
	}
	ctx, cancel := context.WithTimeout(context.Background(), 35*time.Second)
	defer cancel()
	c, err := client.DialContext(ctx, client.Options{HostPort: selected.Endpoint, Namespace: namespace,
		Identity: "celetixo-coordination-probe", ConnectionOptions: client.ConnectionOptions{TLS: transport,
			DialOptions: []grpc.DialOption{grpc.WithUnaryInterceptor(rpcStatus)}}, Logger: quietLogger{}})
	if err != nil {
		return errors.New("SDK connection")
	}
	defer c.Close()
	if mode == "run-worker" {
		// Cache-free tasks force replay and avoid an additional sticky queue name.
		worker.SetStickyWorkflowCacheSize(0)
		w := worker.New(c, selected.TaskQueue, worker.Options{MaxConcurrentActivityExecutionSize: 2,
			MaxConcurrentWorkflowTaskExecutionSize: 2, MaxConcurrentActivityTaskPollers: 1,
			MaxConcurrentWorkflowTaskPollers: 2, WorkerStopTimeout: 5 * time.Second})
		w.RegisterWorkflowWithOptions(probeWorkflow, workflow.RegisterOptions{Name: probeType})
		w.RegisterActivityWithOptions(probeActivity, activity.RegisterOptions{Name: probeActivityType})
		if err := w.Start(); err != nil {
			return errors.New("SDK worker start")
		}
		if err := report("sdk-worker-ready", 1); err != nil {
			w.Stop()
			return err
		}
		shutdown := make(chan os.Signal, 1)
		signal.Notify(shutdown, syscall.SIGTERM, syscall.SIGINT)
		defer signal.Stop(shutdown)
		<-shutdown
		w.Stop()
		return report("sdk-worker-stopped", 1)
	}
	switch mode {
	case "start-probe":
		canonical, err := operatorfile.Read(selected.ProbeIntentFile)
		if err != nil {
			return errors.New("probe canonical input")
		}
		if _, err := probeActivity(ctx, canonical); err != nil {
			return err
		}
		run, err := c.ExecuteWorkflow(ctx, client.StartWorkflowOptions{ID: selected.ProbeWorkflowID, TaskQueue: selected.TaskQueue,
			WorkflowExecutionTimeout: 5 * time.Minute, WorkflowTaskTimeout: 5 * time.Second,
			WorkflowIDReusePolicy: enums.WORKFLOW_ID_REUSE_POLICY_REJECT_DUPLICATE}, probeType, canonical)
		if err != nil {
			return errors.New("SDK workflow start")
		}
		return json.NewEncoder(os.Stdout).Encode(map[string]any{"component": "temporal-probe", "mode": mode,
			"checks_passed": 1, "workflow_id": run.GetID(), "run_id": run.GetRunID()})
	case "checkpoint-probe":
		deadline := time.Now().Add(25 * time.Second)
		for {
			value, err := c.QueryWorkflow(ctx, selected.ProbeWorkflowID, selected.ProbeRunID, "checkpoint")
			var ready bool
			if err == nil && value.Get(&ready) == nil && ready {
				return report(mode, 1)
			}
			if time.Now().After(deadline) {
				return fmt.Errorf("SDK workflow checkpoint (%s)", status.Code(err))
			}
			time.Sleep(200 * time.Millisecond)
		}
	case "wait-probe":
		if err := c.SignalWorkflow(ctx, selected.ProbeWorkflowID, selected.ProbeRunID, "finish", true); err != nil {
			return errors.New("SDK workflow signal")
		}
		var result string
		if err := c.GetWorkflow(ctx, selected.ProbeWorkflowID, selected.ProbeRunID).Get(ctx, &result); err != nil {
			return errors.New("SDK workflow result")
		}
		canonical, err := operatorfile.Read(selected.ProbeIntentFile)
		if err != nil {
			return errors.New("probe canonical input")
		}
		expected, err := probeActivity(ctx, canonical)
		if err != nil || result != expected {
			return errors.New("SDK canonical intent round trip")
		}
		return report(mode, 2)
	case "replay-probe":
		replayer := worker.NewWorkflowReplayer()
		replayer.RegisterWorkflowWithOptions(probeWorkflow, workflow.RegisterOptions{Name: probeType})
		description, err := c.DescribeWorkflowExecution(ctx, selected.ProbeWorkflowID, selected.ProbeRunID)
		if err != nil || description.GetWorkflowExecutionInfo().GetStatus() != enums.WORKFLOW_EXECUTION_STATUS_COMPLETED {
			return errors.New("completed workflow description")
		}
		if err := replayer.ReplayWorkflowExecution(ctx, c.WorkflowService(), quietLogger{}, namespace,
			workflow.Execution{ID: selected.ProbeWorkflowID, RunID: selected.ProbeRunID}); err != nil {
			return errors.New("recorded workflow replay")
		}
		return report(mode, 1)
	}
	return errors.New("coordination mode")
}

// Expose bounded API/status classifications without request bodies, raw errors,
// headers, credentials or dependency connection details.
func rpcStatus(ctx context.Context, method string, request, response any, connection *grpc.ClientConn,
	invoke grpc.UnaryInvoker, options ...grpc.CallOption) error {
	err := invoke(ctx, method, request, response, connection, options...)
	if err != nil {
		_ = json.NewEncoder(os.Stderr).Encode(map[string]string{"component": "temporal-sdk",
			"rpc": method, "status": serviceerror.ToStatus(err).Code().String(), "error_type": fmt.Sprintf("%T", err)})
	}
	return err
}

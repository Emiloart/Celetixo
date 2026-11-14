// The probe operates only a dedicated private fixture. It validates coordination
// authentication/authorization; it does not invoke or infer an Engine commit.
package main

import (
	"bytes"
	"context"
	"crypto/rand"
	"crypto/tls"
	"crypto/x509"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"os"
	"time"

	platform "github.com/Emiloart/Celetixo/services/orchestration/gen/platformv1"
	"github.com/Emiloart/Celetixo/services/orchestration/operatorfile"
	"github.com/Emiloart/Celetixo/services/orchestration/temporalauth"
	common "go.temporal.io/api/common/v1"
	queues "go.temporal.io/api/taskqueue/v1"
	workflow "go.temporal.io/api/workflowservice/v1"
	"google.golang.org/grpc"
	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/credentials"
	"google.golang.org/grpc/status"
	"google.golang.org/protobuf/types/known/durationpb"
)

type configuration struct {
	ContractVersion uint32                  `json:"contract_version"`
	Endpoint        string                  `json:"endpoint"`
	ServerName      string                  `json:"server_name"`
	CAFile          string                  `json:"ca_file"`
	Certificate     string                  `json:"certificate"`
	PrivateKey      string                  `json:"private_key"`
	Scope           *platform.ScopeSelector `json:"scope"`
	TaskQueue       string                  `json:"task_queue"`
	ProbeWorkflowID string                  `json:"probe_workflow_id,omitempty"`
	ProbeRunID      string                  `json:"probe_run_id,omitempty"`
	ProbeIntentFile string                  `json:"probe_intent_file,omitempty"`
}

func execute() error {
	if len(os.Args) != 4 || os.Args[2] != "--config" {
		return errors.New("configuration")
	}
	mode := os.Args[1]
	if mode != "setup" && mode != "worker" && mode != "client" && mode != "unmapped" && mode != "health" &&
		mode != "run-worker" && mode != "start-probe" && mode != "checkpoint-probe" && mode != "wait-probe" && mode != "replay-probe" {
		return errors.New("mode")
	}
	data, err := operatorfile.Read(os.Args[3])
	if err != nil {
		return err
	}
	var selected configuration
	decoder := json.NewDecoder(bytes.NewReader(data))
	decoder.DisallowUnknownFields()
	if err := decoder.Decode(&selected); err != nil {
		return err
	}
	if err := decoder.Decode(new(any)); err != io.EOF || selected.ContractVersion != 1 || selected.TaskQueue == "" || selected.ServerName == "" {
		return errors.New("configuration")
	}
	namespace, err := temporalauth.Namespace(selected.Scope)
	if err != nil {
		return err
	}
	ca, err := operatorfile.Read(selected.CAFile)
	if err != nil {
		return err
	}
	certificate, err := operatorfile.Read(selected.Certificate)
	if err != nil {
		return err
	}
	key, err := operatorfile.Read(selected.PrivateKey)
	if err != nil {
		return err
	}
	identity, err := tls.X509KeyPair(certificate, key)
	if err != nil {
		return err
	}
	roots := x509.NewCertPool()
	if !roots.AppendCertsFromPEM(ca) {
		return errors.New("trust")
	}
	transport := &tls.Config{MinVersion: tls.VersionTLS13, RootCAs: roots, Certificates: []tls.Certificate{identity}, ServerName: selected.ServerName}
	connection, err := grpc.NewClient(selected.Endpoint,
		grpc.WithTransportCredentials(credentials.NewTLS(transport)),
		grpc.WithDefaultCallOptions(grpc.MaxCallSendMsgSize(1<<20), grpc.MaxCallRecvMsgSize(1<<20)))
	if err != nil {
		return err
	}
	defer connection.Close()
	client := workflow.NewWorkflowServiceClient(connection)
	deadline := time.Now().Add(40 * time.Second)
	for {
		ctx, cancel := context.WithTimeout(context.Background(), 2*time.Second)
		_, err = client.GetSystemInfo(ctx, &workflow.GetSystemInfoRequest{})
		cancel()
		if mode == "unmapped" {
			if status.Code(err) != codes.PermissionDenied {
				return errors.New("unmapped identity admitted or transport did not reach authorization")
			}
			return report(mode, 1)
		}
		if err == nil {
			break
		}
		if (status.Code(err) != codes.Unavailable && status.Code(err) != codes.DeadlineExceeded) || time.Now().After(deadline) {
			return errors.New("readiness")
		}
		time.Sleep(200 * time.Millisecond)
	}
	if mode == "run-worker" || mode == "start-probe" || mode == "checkpoint-probe" || mode == "wait-probe" || mode == "replay-probe" {
		return coordinationProbe(mode, &selected, namespace, transport)
	}
	ctx, cancel := context.WithTimeout(context.Background(), 8*time.Second)
	defer cancel()
	switch mode {
	case "setup":
		_, err := client.RegisterNamespace(ctx, &workflow.RegisterNamespaceRequest{Namespace: namespace,
			WorkflowExecutionRetentionPeriod: durationpb.New(24 * time.Hour)})
		if err != nil && status.Code(err) != codes.AlreadyExists {
			return errors.New("namespace registration")
		}
		_, err = client.DescribeNamespace(ctx, &workflow.DescribeNamespaceRequest{Namespace: namespace})
		if err != nil {
			return errors.New("namespace lookup")
		}
		return report(mode, 2)
	case "worker":
		_, err := client.PollWorkflowTaskQueue(ctx, &workflow.PollWorkflowTaskQueueRequest{Namespace: namespace,
			TaskQueue: &queues.TaskQueue{Name: "foreign-queue"}, Identity: "operator"})
		if status.Code(err) != codes.PermissionDenied {
			return errors.New("foreign queue admitted")
		}
		_, err = client.PollWorkflowTaskQueue(ctx, &workflow.PollWorkflowTaskQueueRequest{Namespace: "foreign-namespace",
			TaskQueue: &queues.TaskQueue{Name: selected.TaskQueue}, Identity: "operator"})
		if status.Code(err) != codes.PermissionDenied {
			return errors.New("foreign namespace admitted")
		}
		_, err = client.RequestCancelWorkflowExecution(ctx, &workflow.RequestCancelWorkflowExecutionRequest{Namespace: namespace,
			WorkflowExecution: &common.WorkflowExecution{WorkflowId: "unowned"}})
		if status.Code(err) != codes.PermissionDenied {
			return errors.New("worker acquired client authority")
		}
		// The pinned server requires a long-poll deadline of at least two
		// seconds. Keep the probe bounded while reaching the actual poll path.
		pollCtx, pollCancel := context.WithTimeout(context.Background(), 5*time.Second)
		defer pollCancel()
		_, err = client.PollWorkflowTaskQueue(pollCtx, &workflow.PollWorkflowTaskQueueRequest{Namespace: namespace,
			TaskQueue: &queues.TaskQueue{Name: selected.TaskQueue}, Identity: "probe-worker"})
		if err != nil && status.Code(err) != codes.DeadlineExceeded {
			return errors.New("authorized queue rejected")
		}
		return report(mode, 4)
	case "client":
		_, err := client.PollActivityTaskQueue(ctx, &workflow.PollActivityTaskQueueRequest{Namespace: namespace,
			TaskQueue: &queues.TaskQueue{Name: selected.TaskQueue}, Identity: "worker"})
		if status.Code(err) != codes.PermissionDenied {
			return errors.New("client acquired worker authority")
		}
		var random [16]byte
		if _, err := rand.Read(random[:]); err != nil {
			return err
		}
		id := "auth-probe-" + hex.EncodeToString(random[:])
		request := &workflow.StartWorkflowExecutionRequest{Namespace: namespace, WorkflowId: id,
			WorkflowType: &common.WorkflowType{Name: "celetixo.fixture.pending"}, TaskQueue: &queues.TaskQueue{Name: selected.TaskQueue},
			WorkflowExecutionTimeout: durationpb.New(10 * time.Second), RequestId: id, RequestEagerExecution: true}
		_, err = client.StartWorkflowExecution(ctx, request)
		if status.Code(err) != codes.PermissionDenied {
			return errors.New("client acquired eager worker task")
		}
		request.RequestEagerExecution = false
		started, err := client.StartWorkflowExecution(ctx, request)
		if err != nil || started.GetRunId() == "" {
			return errors.New("authorized workflow start")
		}
		execution := &common.WorkflowExecution{WorkflowId: id, RunId: started.RunId}
		if _, err := client.DescribeWorkflowExecution(ctx, &workflow.DescribeWorkflowExecutionRequest{Namespace: namespace, Execution: execution}); err != nil {
			return errors.New("durable workflow lookup")
		}
		if _, err := client.RequestCancelWorkflowExecution(ctx, &workflow.RequestCancelWorkflowExecutionRequest{Namespace: namespace, WorkflowExecution: execution, RequestId: id}); err != nil {
			return errors.New("authorized cancellation")
		}
		return report(mode, 5)
	default:
		return report(mode, 1)
	}
}

func report(mode string, checks int) error {
	return json.NewEncoder(os.Stdout).Encode(map[string]any{"component": "temporal-probe", "mode": mode, "checks_passed": checks})
}

func main() {
	if err := execute(); err != nil {
		fmt.Fprintln(os.Stderr, "temporal-probe: failed at", err)
		os.Exit(1)
	}
}

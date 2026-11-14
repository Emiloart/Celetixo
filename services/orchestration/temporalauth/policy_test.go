package temporalauth_test

import (
	"context"
	"crypto/sha256"
	"crypto/tls"
	"crypto/x509"
	"encoding/hex"
	"testing"
	"time"

	platform "github.com/Emiloart/Celetixo/services/orchestration/gen/platformv1"
	"github.com/Emiloart/Celetixo/services/orchestration/temporalauth"
	commands "go.temporal.io/api/command/v1"
	enums "go.temporal.io/api/enums/v1"
	queues "go.temporal.io/api/taskqueue/v1"
	worker "go.temporal.io/api/worker/v1"
	workflow "go.temporal.io/api/workflowservice/v1"
	"go.temporal.io/server/common/authorization"
	"google.golang.org/grpc/credentials"
)

const prefix = "/temporal.api.workflowservice.v1.WorkflowService/"

type harness struct {
	policy    *temporalauth.Policy
	now       time.Time
	scope     *platform.ScopeSelector
	namespace string
	info      *authorization.AuthInfo
}

// This tests the plugin using the transport state supplied by Temporal. It does
// not claim a real Temporal listener or TLS handshake; those require integration.
func setup(t *testing.T, role temporalauth.Role) *harness {
	t.Helper()
	h := &harness{now: time.Unix(1794640000, 0), scope: &platform.ScopeSelector{OrganizationId: "organization", WorkspaceId: "workspace", RepositoryId: "repository"}}
	leaf := &x509.Certificate{Raw: []byte("synthetic peer certificate"), NotBefore: h.now.Add(-time.Hour), NotAfter: h.now.Add(time.Hour)}
	digest := sha256.Sum256(leaf.Raw)
	binding := temporalauth.Binding{CertificateSHA256: hex.EncodeToString(digest[:]), Principal: "principal", Scope: h.scope, Role: role, TaskQueues: []string{"engine"}}
	if role == temporalauth.Operator {
		binding.Scope = nil
		binding.TaskQueues = nil
	}
	var err error
	h.policy, err = temporalauth.New([]temporalauth.Binding{binding}, func() time.Time { return h.now })
	if err != nil {
		t.Fatal(err)
	}
	h.namespace, err = temporalauth.Namespace(h.scope)
	if err != nil {
		t.Fatal(err)
	}
	h.info = &authorization.AuthInfo{TLSConnection: &credentials.TLSInfo{State: tls.ConnectionState{Version: tls.VersionTLS13, HandshakeComplete: true, PeerCertificates: []*x509.Certificate{leaf}, VerifiedChains: [][]*x509.Certificate{{leaf}}}}}
	return h
}

func (h *harness) allowed(t *testing.T, claims *authorization.Claims, method, namespace string, request any) bool {
	t.Helper()
	result, err := h.policy.Authorize(context.Background(), claims, &authorization.CallTarget{APIName: prefix + method, Namespace: namespace, Request: request})
	if err != nil {
		t.Fatal(err)
	}
	return result.Decision == authorization.DecisionAllow
}

func TestUnmappedUnverifiedExpiredAndAlternativeCredentialsFailClosed(t *testing.T) {
	h := setup(t, temporalauth.Worker)
	if _, err := h.policy.GetClaims(h.info); err != nil {
		t.Fatal(err)
	}
	for _, bad := range []*authorization.AuthInfo{nil, {}, {TLSConnection: &credentials.TLSInfo{}}, {TLSConnection: h.info.TLSConnection, AuthToken: "untrusted"}} {
		if _, err := h.policy.GetClaims(bad); err == nil {
			t.Fatal("unverified identity admitted")
		}
	}
	h.now = h.now.Add(2 * time.Hour)
	if _, err := h.policy.GetClaims(h.info); err == nil {
		t.Fatal("certificate remained usable after expiry on an existing connection")
	}
	h = setup(t, temporalauth.Worker)
	h.info.TLSConnection.State.PeerCertificates[0].Raw = []byte("unmapped peer")
	if _, err := h.policy.GetClaims(h.info); err == nil {
		t.Fatal("unmapped valid-transport identity admitted")
	}
}

func TestNamespaceRoleAndTaskQueueCannotBeSelfGranted(t *testing.T) {
	h := setup(t, temporalauth.Worker)
	claims, err := h.policy.GetClaims(h.info)
	if err != nil {
		t.Fatal(err)
	}
	request := &workflow.PollWorkflowTaskQueueRequest{Namespace: h.namespace, TaskQueue: &queues.TaskQueue{Name: "engine"}}
	if !h.allowed(t, claims, "PollWorkflowTaskQueue", h.namespace, request) {
		t.Fatal("bound worker could not poll its queue")
	}
	claims.System = authorization.RoleAdmin
	claims.Namespaces["foreign"] = authorization.RoleAdmin
	if h.allowed(t, claims, "PollWorkflowTaskQueue", "foreign", request) || h.allowed(t, claims, "RequestCancelWorkflowExecution", h.namespace, &workflow.RequestCancelWorkflowExecutionRequest{Namespace: h.namespace}) {
		t.Fatal("claims fields bypassed registry scope/role")
	}
	request.TaskQueue.Name = "foreign-queue"
	if h.allowed(t, claims, "PollWorkflowTaskQueue", h.namespace, request) {
		t.Fatal("foreign task queue admitted")
	}
	request.TaskQueue = &queues.TaskQueue{Name: "worker-generated-sticky", Kind: enums.TASK_QUEUE_KIND_STICKY, NormalName: "engine"}
	if h.allowed(t, claims, "PollWorkflowTaskQueue", h.namespace, request) {
		t.Fatal("caller-supplied normal queue name authorized an unverified sticky queue")
	}
	request.TaskQueue.NormalName = "foreign-queue"
	if h.allowed(t, claims, "PollWorkflowTaskQueue", h.namespace, request) {
		t.Fatal("sticky queue bypassed its normal queue policy")
	}
	forged := &authorization.Claims{Subject: claims.Subject, System: authorization.RoleAdmin, AuthType: "mtls"}
	if h.allowed(t, forged, "GetSystemInfo", "", &workflow.GetSystemInfoRequest{}) {
		t.Fatal("self-issued claims admitted")
	}
	other := setup(t, temporalauth.Operator)
	if other.allowed(t, claims, "GetSystemInfo", "", &workflow.GetSystemInfoRequest{}) {
		t.Fatal("claims from a different policy instance admitted")
	}
}

func TestWorkflowCommandsCannotBypassNamespaceAndQueuePolicy(t *testing.T) {
	h := setup(t, temporalauth.Worker)
	claims, err := h.policy.GetClaims(h.info)
	if err != nil {
		t.Fatal(err)
	}
	attributes := &commands.StartChildWorkflowExecutionCommandAttributes{Namespace: h.namespace, TaskQueue: &queues.TaskQueue{Name: "engine"}}
	command := &commands.Command{CommandType: enums.COMMAND_TYPE_START_CHILD_WORKFLOW_EXECUTION, Attributes: &commands.Command_StartChildWorkflowExecutionCommandAttributes{StartChildWorkflowExecutionCommandAttributes: attributes}}
	request := &workflow.RespondWorkflowTaskCompletedRequest{Namespace: h.namespace, Commands: []*commands.Command{command}}
	if !h.allowed(t, claims, "RespondWorkflowTaskCompleted", h.namespace, request) {
		t.Fatal("bound child workflow denied")
	}
	attributes.Namespace = "foreign"
	if h.allowed(t, claims, "RespondWorkflowTaskCompleted", h.namespace, request) {
		t.Fatal("child workflow crossed namespace policy")
	}
	attributes.Namespace = h.namespace
	attributes.TaskQueue.Name = "foreign-queue"
	if h.allowed(t, claims, "RespondWorkflowTaskCompleted", h.namespace, request) {
		t.Fatal("child workflow crossed queue policy")
	}
	command.CommandType = enums.COMMAND_TYPE_COMPLETE_WORKFLOW_EXECUTION
	if h.allowed(t, claims, "RespondWorkflowTaskCompleted", h.namespace, request) {
		t.Fatal("mismatched command tag/attributes hid an external action")
	}
	command.CommandType = enums.CommandType(999)
	if h.allowed(t, claims, "RespondWorkflowTaskCompleted", h.namespace, request) {
		t.Fatal("future unsupported command admitted")
	}
}

func TestNamespaceIdentityAndPolicyValidation(t *testing.T) {
	a := &platform.ScopeSelector{OrganizationId: "ab", WorkspaceId: "c", RepositoryId: "d"}
	b := &platform.ScopeSelector{OrganizationId: "a", WorkspaceId: "bc", RepositoryId: "d"}
	x, _ := temporalauth.Namespace(a)
	y, _ := temporalauth.Namespace(b)
	if x == y {
		t.Fatal("scope tuples have ambiguous namespace identity")
	}
	if _, err := temporalauth.New(nil, nil); err == nil {
		t.Fatal("empty identity policy admitted")
	}
	invalid := temporalauth.Binding{CertificateSHA256: "invalid", Principal: "principal", Role: temporalauth.Operator}
	if _, err := temporalauth.New([]temporalauth.Binding{invalid}, nil); err == nil {
		t.Fatal("invalid certificate binding admitted")
	}
}

func TestWorkerHeartbeatCannotReportForeignQueues(t *testing.T) {
	h := setup(t, temporalauth.Worker)
	claims, err := h.policy.GetClaims(h.info)
	if err != nil {
		t.Fatal(err)
	}
	request := &workflow.RecordWorkerHeartbeatRequest{Namespace: h.namespace,
		WorkerHeartbeat: []*worker.WorkerHeartbeat{{TaskQueue: "engine", WorkerIdentity: "operator"}}}
	if !h.allowed(t, claims, "RecordWorkerHeartbeat", h.namespace, request) {
		t.Fatal("bound worker heartbeat denied")
	}
	request.WorkerHeartbeat[0].TaskQueue = "foreign"
	if h.allowed(t, claims, "RecordWorkerHeartbeat", h.namespace, request) {
		t.Fatal("foreign queue heartbeat admitted")
	}
	request.WorkerHeartbeat = []*worker.WorkerHeartbeat{nil}
	if h.allowed(t, claims, "RecordWorkerHeartbeat", h.namespace, request) {
		t.Fatal("unbound heartbeat admitted")
	}
}

func TestClientCannotPollOrAcquireEagerWorkerTasks(t *testing.T) {
	h := setup(t, temporalauth.Client)
	claims, err := h.policy.GetClaims(h.info)
	if err != nil {
		t.Fatal(err)
	}
	start := &workflow.StartWorkflowExecutionRequest{Namespace: h.namespace, TaskQueue: &queues.TaskQueue{Name: "engine"}}
	if !h.allowed(t, claims, "StartWorkflowExecution", h.namespace, start) {
		t.Fatal("bound client could not start work")
	}
	poll := &workflow.PollActivityTaskQueueRequest{Namespace: h.namespace, TaskQueue: &queues.TaskQueue{Name: "engine"}}
	if h.allowed(t, claims, "PollActivityTaskQueue", h.namespace, poll) {
		t.Fatal("client acquired worker polling authority")
	}
	start.RequestEagerExecution = true
	if h.allowed(t, claims, "StartWorkflowExecution", h.namespace, start) {
		t.Fatal("client acquired an eager workflow task outside the worker boundary")
	}
}

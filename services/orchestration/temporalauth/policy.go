// Package temporalauth implements Temporal's authentication/authorization plugin
// interfaces. It grants workflow coordination access only; Engine permissions,
// evidence and transaction decisions remain separate authority boundaries.
package temporalauth

import (
	"context"
	"crypto/sha256"
	"crypto/tls"
	"encoding/binary"
	"encoding/hex"
	"errors"
	"strings"
	"time"
	"unicode"
	"unicode/utf8"

	platform "github.com/Emiloart/Celetixo/services/orchestration/gen/platformv1"
	common "go.temporal.io/api/common/v1"
	enums "go.temporal.io/api/enums/v1"
	queues "go.temporal.io/api/taskqueue/v1"
	workflow "go.temporal.io/api/workflowservice/v1"
	"go.temporal.io/server/common/authorization"
)

type Role uint8

const (
	Operator Role = iota + 1
	Client
	Worker
)

// Binding is trusted operator input. A certificate has exactly one explicit
// binding. Namespace names derive from all three scope IDs; there is no global
// organization default. Operator is a separate, explicitly privileged identity.
type Binding struct {
	CertificateSHA256 string                  `json:"certificate_sha256"`
	Principal         string                  `json:"principal"`
	Scope             *platform.ScopeSelector `json:"scope"`
	Role              Role                    `json:"role"`
	TaskQueues        []string                `json:"task_queues"`
}

type entry struct {
	principal string
	namespace string
	role      Role
	queues    map[string]bool
}
type Policy struct {
	entries map[[32]byte]entry
	clock   func() time.Time
}
type proof struct {
	policy      *Policy
	fingerprint [32]byte
}

var ErrPolicy = errors.New("invalid bounded Temporal identity policy")
var ErrIdentity = errors.New("Temporal client identity is not authorized")

func identity(value string) bool {
	return value != "" && len(value) <= 256 && utf8.ValidString(value) &&
		!strings.ContainsFunc(value, func(ch rune) bool { return unicode.IsControl(ch) || unicode.IsSpace(ch) })
}

// Namespace deterministically scopes Temporal metadata; knowing this name grants
// no access. Length-prefixing prevents ambiguous organization/workspace tuples.
func Namespace(scope *platform.ScopeSelector) (string, error) {
	if scope == nil {
		return "", ErrPolicy
	}
	fields := []string{scope.OrganizationId, scope.WorkspaceId, scope.RepositoryId}
	data := []byte("celetixo/temporal-namespace/v1")
	for _, field := range fields {
		if !identity(field) {
			return "", ErrPolicy
		}
		data = binary.BigEndian.AppendUint64(data, uint64(len(field)))
		data = append(data, field...)
	}
	digest := sha256.Sum256(data)
	return "celetixo-" + hex.EncodeToString(digest[:]), nil
}

// New makes an immutable policy. The clock is an embedding dependency for tests;
// nil uses the host authentication clock. Callers cannot supply it over an API.
func New(bindings []Binding, clock func() time.Time) (*Policy, error) {
	if len(bindings) == 0 || len(bindings) > 512 {
		return nil, ErrPolicy
	}
	if clock == nil {
		clock = time.Now
	}
	p := &Policy{entries: make(map[[32]byte]entry, len(bindings)), clock: clock}
	for _, b := range bindings {
		fingerprint, err := hex.DecodeString(b.CertificateSHA256)
		if err != nil || len(fingerprint) != 32 || hex.EncodeToString(fingerprint) != b.CertificateSHA256 || !identity(b.Principal) {
			return nil, ErrPolicy
		}
		var key [32]byte
		copy(key[:], fingerprint)
		if _, exists := p.entries[key]; exists {
			return nil, ErrPolicy
		}
		e := entry{principal: b.Principal, role: b.Role, queues: map[string]bool{}}
		switch b.Role {
		case Operator:
			if b.Scope != nil || len(b.TaskQueues) != 0 {
				return nil, ErrPolicy
			}
		case Client, Worker:
			e.namespace, err = Namespace(b.Scope)
			if err != nil || len(b.TaskQueues) == 0 || len(b.TaskQueues) > 16 {
				return nil, ErrPolicy
			}
			for _, queue := range b.TaskQueues {
				if !identity(queue) || e.queues[queue] {
					return nil, ErrPolicy
				}
				e.queues[queue] = true
			}
		default:
			return nil, ErrPolicy
		}
		p.entries[key] = e
	}
	return p, nil
}

var _ authorization.ClaimMapper = (*Policy)(nil)
var _ authorization.Authorizer = (*Policy)(nil)

func (p *Policy) GetClaims(info *authorization.AuthInfo) (*authorization.Claims, error) {
	if info == nil || info.TLSConnection == nil || info.AuthToken != "" || info.ExtraData != "" {
		return nil, ErrIdentity
	}
	state := &info.TLSConnection.State
	if !state.HandshakeComplete || state.Version < tls.VersionTLS13 || len(state.PeerCertificates) == 0 ||
		len(state.VerifiedChains) == 0 || len(state.VerifiedChains[0]) == 0 ||
		state.PeerCertificates[0] == nil || state.VerifiedChains[0][0] == nil ||
		!state.PeerCertificates[0].Equal(state.VerifiedChains[0][0]) {
		return nil, ErrIdentity
	}
	now := p.clock()
	for _, cert := range state.VerifiedChains[0] {
		if cert == nil || now.Before(cert.NotBefore) || !now.Before(cert.NotAfter) {
			return nil, ErrIdentity
		}
	}
	key := sha256.Sum256(state.PeerCertificates[0].Raw)
	e, ok := p.entries[key]
	if !ok {
		return nil, ErrIdentity
	}
	claims := &authorization.Claims{Subject: e.principal, AuthType: "mtls", Extensions: proof{p, key}}
	if e.role == Operator {
		claims.System = authorization.RoleAdmin
	} else {
		// Namespace claims support Temporal's own checks; our Authorizer applies
		// the stricter method/queue policy using the private registry entry.
		claims.Namespaces = map[string]authorization.Role{e.namespace: authorization.RoleReader | authorization.RoleWriter}
	}
	return claims, nil
}

const prefix = "/temporal.api.workflowservice.v1.WorkflowService/"

var clientMethods = map[string]bool{
	"StartWorkflowExecution": true, "DescribeWorkflowExecution": true,
	"GetWorkflowExecutionHistory": true, "RequestCancelWorkflowExecution": true,
	"SignalWorkflowExecution": true, "QueryWorkflow": true,
	"UpdateWorkflowExecution": true, "PollWorkflowExecutionUpdate": true,
	"DescribeNamespace": true, "DescribeTaskQueue": true,
}
var workerMethods = map[string]bool{
	"GetWorkflowExecutionHistory": true,
	"PollWorkflowTaskQueue":       true, "PollActivityTaskQueue": true,
	"RespondWorkflowTaskCompleted": true, "RespondWorkflowTaskFailed": true,
	"RespondQueryTaskCompleted":    true,
	"RecordWorkerHeartbeat":        true,
	"RespondActivityTaskCompleted": true, "RespondActivityTaskFailed": true,
	"RespondActivityTaskCanceled": true, "RecordActivityTaskHeartbeat": true,
	"ResetStickyTaskQueue": true, "DescribeNamespace": true, "DescribeTaskQueue": true,
}

func (e entry) queue(q *queues.TaskQueue, inherit bool) bool {
	if q == nil || q.Name == "" {
		return inherit
	}
	if q.Kind == enums.TASK_QUEUE_KIND_STICKY {
		// NormalName is caller-supplied metadata, not proof that this sticky
		// queue belongs to an authorized normal queue. This bounded profile
		// requires cache-free workers until a verified association is available.
		return false
	}
	return (q.Kind == enums.TASK_QUEUE_KIND_NORMAL || q.Kind == enums.TASK_QUEUE_KIND_UNSPECIFIED) && e.queues[q.Name]
}

func (e entry) namespaceOrCurrent(value string) bool { return value == "" || value == e.namespace }

// Commands can schedule work without a separate frontend RPC. Check their
// namespace/queue fields too; checking only Poll/Start would leave a bypass.
func (e entry) commands(request *workflow.RespondWorkflowTaskCompletedRequest) bool {
	if request.StickyAttributes != nil && !e.queue(request.StickyAttributes.WorkerTaskQueue, false) {
		return false
	}
	for _, command := range request.Commands {
		if command == nil {
			return false
		}
		message := command.ProtoReflect()
		attributes := message.Descriptor().Oneofs().ByName("attributes")
		value := command.CommandType.Descriptor().Values().ByNumber(command.CommandType.Number())
		if attributes == nil || value == nil {
			return false
		}
		selected := message.WhichOneof(attributes)
		expected := strings.ToLower(strings.TrimPrefix(string(value.Name()), "COMMAND_TYPE_")) + "_command_attributes"
		if selected == nil || string(selected.Name()) != expected {
			return false
		}
		switch command.CommandType {
		case enums.COMMAND_TYPE_SCHEDULE_ACTIVITY_TASK:
			a := command.GetScheduleActivityTaskCommandAttributes()
			if a == nil || !e.queue(a.TaskQueue, true) {
				return false
			}
		case enums.COMMAND_TYPE_START_CHILD_WORKFLOW_EXECUTION:
			a := command.GetStartChildWorkflowExecutionCommandAttributes()
			if a == nil || !e.namespaceOrCurrent(a.Namespace) || !e.queue(a.TaskQueue, true) {
				return false
			}
		case enums.COMMAND_TYPE_CONTINUE_AS_NEW_WORKFLOW_EXECUTION:
			a := command.GetContinueAsNewWorkflowExecutionCommandAttributes()
			if a == nil || !e.queue(a.TaskQueue, true) {
				return false
			}
		case enums.COMMAND_TYPE_REQUEST_CANCEL_EXTERNAL_WORKFLOW_EXECUTION:
			a := command.GetRequestCancelExternalWorkflowExecutionCommandAttributes()
			if a == nil || !e.namespaceOrCurrent(a.Namespace) {
				return false
			}
		case enums.COMMAND_TYPE_SIGNAL_EXTERNAL_WORKFLOW_EXECUTION:
			a := command.GetSignalExternalWorkflowExecutionCommandAttributes()
			if a == nil || !e.namespaceOrCurrent(a.Namespace) {
				return false
			}
		case enums.COMMAND_TYPE_REQUEST_CANCEL_ACTIVITY_TASK, enums.COMMAND_TYPE_START_TIMER,
			enums.COMMAND_TYPE_COMPLETE_WORKFLOW_EXECUTION, enums.COMMAND_TYPE_FAIL_WORKFLOW_EXECUTION,
			enums.COMMAND_TYPE_CANCEL_TIMER, enums.COMMAND_TYPE_CANCEL_WORKFLOW_EXECUTION,
			enums.COMMAND_TYPE_RECORD_MARKER, enums.COMMAND_TYPE_UPSERT_WORKFLOW_SEARCH_ATTRIBUTES,
			enums.COMMAND_TYPE_PROTOCOL_MESSAGE, enums.COMMAND_TYPE_MODIFY_WORKFLOW_PROPERTIES:
		default:
			return false
		}
	}
	return true
}

func (p *Policy) Authorize(_ context.Context, claims *authorization.Claims, target *authorization.CallTarget) (authorization.Result, error) {
	deny := authorization.Result{Decision: authorization.DecisionDeny}
	if claims == nil || target == nil || target.Request == nil {
		return deny, nil
	}
	bound, ok := claims.Extensions.(proof)
	if !ok || bound.policy != p {
		return deny, nil
	}
	e, ok := p.entries[bound.fingerprint]
	if !ok || claims.Subject != e.principal || claims.AuthType != "mtls" {
		return deny, nil
	}
	allow := authorization.Result{Decision: authorization.DecisionAllow,
		Principal: &common.Principal{Type: "mtls", Name: e.principal}}
	if e.role == Operator {
		return allow, nil
	}
	if target.APIName == prefix+"GetSystemInfo" || target.APIName == "/grpc.health.v1.Health/Check" {
		return allow, nil
	}
	if target.Namespace != e.namespace || !strings.HasPrefix(target.APIName, prefix) {
		return deny, nil
	}
	if request, ok := target.Request.(interface{ GetNamespace() string }); !ok || request.GetNamespace() != target.Namespace {
		return deny, nil
	}
	method := strings.TrimPrefix(target.APIName, prefix)
	if (e.role == Client && !clientMethods[method]) || (e.role == Worker && !workerMethods[method]) {
		return deny, nil
	}
	if request, ok := target.Request.(interface{ GetTaskQueue() *queues.TaskQueue }); ok && !e.queue(request.GetTaskQueue(), false) {
		return deny, nil
	}
	if request, ok := target.Request.(*workflow.StartWorkflowExecutionRequest); ok && request.RequestEagerExecution {
		return deny, nil
	}
	if request, ok := target.Request.(*workflow.RespondWorkflowTaskCompletedRequest); ok && !e.commands(request) {
		return deny, nil
	}
	if request, ok := target.Request.(*workflow.RecordWorkerHeartbeatRequest); ok {
		if len(request.WorkerHeartbeat) == 0 || len(request.WorkerHeartbeat) > 16 {
			return deny, nil
		}
		for _, heartbeat := range request.WorkerHeartbeat {
			if heartbeat == nil || !e.queues[heartbeat.TaskQueue] {
				return deny, nil
			}
		}
	}
	return allow, nil
}

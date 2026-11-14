package protocol

import (
	"bytes"
	"errors"
	"os"
	"testing"

	platform "github.com/Emiloart/Celetixo/services/orchestration/gen/platformv1"
	"google.golang.org/protobuf/encoding/protowire"
	"google.golang.org/protobuf/proto"
)

func golden(t testing.TB) []byte {
	t.Helper()
	data, err := os.ReadFile("../../../proto/fixtures/task_intent_envelope.bin")
	if err != nil {
		t.Fatal(err)
	}
	return data
}

func TestCanonicalTaskIntentGoldenPreservesCompleteADRMessage(t *testing.T) {
	data := golden(t)
	var envelope platform.TaskIntentEnvelope
	if err := Decode(data, &envelope); err != nil {
		t.Fatal(err)
	}
	intent := envelope.GetIntent()
	if intent == nil || intent.ProtoReflect().Descriptor().Fields().Len() != 16 ||
		intent.GetImpactPolicy() == nil || intent.GetVerification() == nil ||
		intent.GetBudget() == nil || intent.GetRetryPolicy() == nil ||
		intent.GetRollbackPolicy() == nil || intent.GetEscalationPolicy() == nil || intent.GetProvenance() == nil {
		t.Fatal("generated consumer lost canonical TaskIntent semantics")
	}
	encoded, err := (proto.MarshalOptions{Deterministic: true}).Marshal(&envelope)
	if err != nil || !bytes.Equal(data, encoded) {
		t.Fatal("Go encoding differs from the protoc/Rust golden bytes", err)
	}
}

func TestOriginalWireRejectsUnknownNestedDuplicateOverflowAndNoncanonicalFields(t *testing.T) {
	data := golden(t)
	var envelope platform.TaskIntentEnvelope
	if err := Decode(data, &envelope); err != nil {
		t.Fatal(err)
	}
	unknown := proto.Clone(&envelope).(*platform.TaskIntentEnvelope)
	unknown.Intent.Verification.ProtoReflect().SetUnknown([]byte{0xf8, 0x07, 0x01})
	nested, _ := proto.Marshal(unknown)
	duplicate := append(bytes.Clone(data), 0x08, 0x01)
	invalidBool := proto.Clone(&envelope).(*platform.TaskIntentEnvelope)
	// A duplicate bool is rejected before protobuf's last-value merge.
	invalidBool.Intent.Verification.ProtoReflect().SetUnknown([]byte{0x18, 0x02})
	boolBytes, _ := proto.Marshal(invalidBool)
	for name, input := range map[string][]byte{
		"nested unknown": nested, "duplicate singular": duplicate,
		"noncanonical varint": {0x08, 0x81, 0x00}, "overflow uint32": {0x08, 0xff, 0xff, 0xff, 0xff, 0x1f},
		"invalid bool": boolBytes, "invalid tag": {0}, "truncated string": {0x2a, 0x04, 'a'},
	} {
		t.Run(name, func(t *testing.T) {
			if err := Decode(input, &platform.TaskIntentEnvelope{}); !errors.Is(err, ErrWire) {
				t.Fatalf("unsafe original wire accepted: %v", err)
			}
		})
	}
}

func TestBudgetsAndClosedLifecycleEnums(t *testing.T) {
	if err := Decode(make([]byte, MaxWireBytes+1), &platform.TaskIntentEnvelope{}); !errors.Is(err, ErrBudget) {
		t.Fatal("payload budget was not enforced")
	}
	var fields []byte
	for range maxFields + 1 {
		fields = protowire.AppendTag(fields, 6, protowire.BytesType)
		fields = protowire.AppendString(fields, "")
	}
	if err := Decode(fields, &platform.TaskIntentEnvelope{}); !errors.Is(err, ErrBudget) {
		t.Fatal("field budget was not enforced", err)
	}
	data, _ := proto.Marshal(&platform.LifecycleEvent{EventKind: platform.LifecycleEventKind(255)})
	if err := Decode(data, &platform.LifecycleEvent{}); !errors.Is(err, ErrWire) {
		t.Fatal("unknown lifecycle enum accepted", err)
	}
}

func FuzzOriginalWire(f *testing.F) {
	f.Add(golden(f))
	f.Add([]byte{0x08, 0x81, 0x00})
	f.Fuzz(func(t *testing.T, data []byte) {
		if len(data) > MaxWireBytes+1 {
			return
		}
		_ = Decode(data, &platform.TaskIntentEnvelope{})
	})
}

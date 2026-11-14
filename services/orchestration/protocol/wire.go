// Package protocol provides bounded wire-shape checks for generated canonical
// messages. It does not authenticate a caller, admit an intent or grant authority.
// Rust Engine intake and current scoped policy remain authoritative.
package protocol

import (
	"errors"
	"math"

	"google.golang.org/protobuf/encoding/protowire"
	"google.golang.org/protobuf/proto"
	"google.golang.org/protobuf/reflect/protoreflect"
)

const MaxWireBytes = 1 << 20
const maxFields = 32768
const maxDepth = 64

var ErrWire = errors.New("malformed, unsupported or noncanonical protocol shape")
var ErrBudget = errors.New("protocol payload exceeds its bounded wire budget")

// Decode checks original bytes before protobuf can discard unknown semantics or
// merge duplicate singular fields. Field metadata comes from the generated
// descriptor, not a separately maintained schema. Semantic validation is separate.
func Decode(data []byte, target proto.Message) error {
	if len(data) > MaxWireBytes {
		return ErrBudget
	}
	if target == nil || !target.ProtoReflect().IsValid() {
		return ErrWire
	}
	count := 0
	if err := scan(data, target.ProtoReflect().Descriptor(), &count, 0); err != nil {
		return err
	}
	if err := (proto.UnmarshalOptions{RecursionLimit: maxDepth}).Unmarshal(data, target); err != nil {
		return ErrWire
	}
	return nil
}

func integer(data []byte) (uint64, int, error) {
	value, n := protowire.ConsumeVarint(data)
	if n < 0 || protowire.SizeVarint(value) != n {
		return 0, 0, ErrWire
	}
	return value, n, nil
}

func scan(data []byte, descriptor protoreflect.MessageDescriptor, count *int, depth int) error {
	if depth >= maxDepth {
		return ErrBudget
	}
	seen := make(map[protoreflect.FieldNumber]bool)
	for len(data) > 0 {
		*count += 1
		if *count > maxFields {
			return ErrBudget
		}
		key, size, err := integer(data)
		if err != nil || key>>3 == 0 || key>>3 > uint64(protowire.MaxValidNumber) {
			return ErrWire
		}
		data = data[size:]
		number := protoreflect.FieldNumber(key >> 3)
		field := descriptor.Fields().ByNumber(number)
		if field == nil || field.IsMap() || (!field.IsList() && seen[number]) {
			return ErrWire
		}
		seen[number] = true
		switch field.Kind() {
		case protoreflect.StringKind, protoreflect.BytesKind, protoreflect.MessageKind:
			if key&7 != uint64(protowire.BytesType) {
				return ErrWire
			}
			length, n, err := integer(data)
			if err != nil || length > uint64(len(data)-n) {
				return ErrWire
			}
			body := data[n : n+int(length)]
			data = data[n+int(length):]
			if field.Kind() == protoreflect.MessageKind {
				if err := scan(body, field.Message(), count, depth+1); err != nil {
					return err
				}
			}
		case protoreflect.Uint32Kind, protoreflect.Uint64Kind, protoreflect.BoolKind, protoreflect.EnumKind:
			// No repeated numeric field exists in the current contracts. Adding
			// packed fields requires explicit support rather than a silent guess.
			if field.IsList() || key&7 != uint64(protowire.VarintType) {
				return ErrWire
			}
			value, n, err := integer(data)
			if err != nil || (field.Kind() == protoreflect.Uint32Kind && value > math.MaxUint32) ||
				(field.Kind() == protoreflect.BoolKind && value > 1) {
				return ErrWire
			}
			if field.Kind() == protoreflect.EnumKind && (value > math.MaxInt32 ||
				field.Enum().Values().ByNumber(protoreflect.EnumNumber(value)) == nil) {
				return ErrWire
			}
			data = data[n:]
		default:
			return ErrWire
		}
	}
	return nil
}

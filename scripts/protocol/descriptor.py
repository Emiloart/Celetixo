"""Small bounded reader for protoc FileDescriptorSet compatibility metadata.

No repository code is evaluated. The canonical protoc-produced descriptor is
decoded as wire data using the published google.protobuf descriptor field tags.
This helper is tooling, not a second schema source or production wire consumer.
"""


def varint(data, offset):
    value = 0
    for index in range(10):
        if offset >= len(data):
            raise ValueError("truncated descriptor varint")
        byte = data[offset]
        offset += 1
        if index == 9 and byte > 1:
            raise ValueError("descriptor varint overflow")
        value |= (byte & 127) << (index * 7)
        if not byte & 128:
            return value, offset
    raise ValueError("overlong descriptor varint")


def fields(data):
    if len(data) > 8 * 1024 * 1024:
        raise ValueError("descriptor exceeds tooling budget")
    output = {}
    offset = 0
    while offset < len(data):
        key, offset = varint(data, offset)
        number, wire = key >> 3, key & 7
        if not number:
            raise ValueError("zero descriptor field")
        if wire == 0:
            value, offset = varint(data, offset)
        elif wire in (1, 5):
            size = 8 if wire == 1 else 4
            value = data[offset:offset + size]
            offset += size
        elif wire == 2:
            size, offset = varint(data, offset)
            value = data[offset:offset + size]
            offset += size
            if len(value) != size:
                raise ValueError("truncated descriptor field")
        else:
            raise ValueError("unsupported descriptor wire type")
        if offset > len(data):
            raise ValueError("truncated descriptor field")
        output.setdefault(number, []).append(value)
    return output


def one(message, number, default=None):
    values = message.get(number, [])
    if len(values) > 1:
        raise ValueError("duplicate singular descriptor field")
    return values[0] if values else default


def string(message, number, default=""):
    return one(message, number, default.encode()).decode("utf-8")


def field_info(raw):
    value = fields(raw)
    return {
        "name": string(value, 1),
        "number": one(value, 3),
        "label": one(value, 4),
        "type": one(value, 5),
        "type_name": string(value, 6),
        "default_value": string(value, 7),
        "oneof_index": one(value, 9),
        "json_name": string(value, 10),
        "proto3_optional": bool(one(value, 17, 0)),
    }


def enum_info(raw, prefix, output):
    value = fields(raw)
    name = prefix + "." + string(value, 1)
    if name in output:
        raise ValueError("duplicate descriptor enum")
    entries = []
    for raw_entry in value.get(2, []):
        entry = fields(raw_entry)
        number = one(entry, 2)
        if number is None or number > 0x7fffffff:
            raise ValueError("strict intake requires nonnegative enum values")
        entries.append({"name": string(entry, 1), "number": number})
    if not entries or entries[0]["number"] != 0:
        raise ValueError("proto3 enum must start with zero")
    if len({entry["name"] for entry in entries}) != len(entries) or len({entry["number"] for entry in entries}) != len(entries):
        raise ValueError("duplicate enum name/number or alias")
    output[name] = entries


def message_info(raw, prefix, output, enums):
    value = fields(raw)
    name = prefix + "." + string(value, 1)
    if name in output:
        raise ValueError("duplicate descriptor message")
    output[name] = sorted((field_info(item) for item in value.get(2, [])), key=lambda item: item["number"])
    for nested in value.get(3, []):
        message_info(nested, name, output, enums)
    for nested in value.get(4, []):
        enum_info(nested, name, enums)


def summarize(data):
    messages = {}
    enums = {}
    files = []
    for raw in fields(data).get(1, []):
        value = fields(raw)
        files.append({"name": string(value, 1), "package": string(value, 2), "syntax": string(value, 12)})
        if value.get(6):
            raise ValueError("service additions require an expanded explicit compatibility checker")
        for enum in value.get(5, []):
            enum_info(enum, string(value, 2), enums)
        for message in value.get(4, []):
            message_info(message, string(value, 2), messages, enums)
    return {"files": sorted(files, key=lambda item: item["name"]), "messages": dict(sorted(messages.items())), "enums": dict(sorted(enums.items()))}


def compatible(previous, current):
    """v1 is additive only: preserve every published field and exact meaning/type.

    Removal is intentionally stricter than Protobuf binary compatibility because
    ADR TaskIntent semantics may not be dropped, even by reserving old fields.
    """
    for file in previous["files"]:
        if file not in current["files"]:
            raise ValueError("published package/file/syntax changed: " + file["name"])
    for name, entries in current.get("enums", {}).items():
        by_number = {entry["number"]: entry for entry in entries}
        by_name = {entry["name"]: entry for entry in entries}
        if len(by_number) != len(entries) or len(by_name) != len(entries):
            raise ValueError("duplicate enum name/number: " + name)
    for name, entries in previous.get("enums", {}).items():
        if name not in current.get("enums", {}):
            raise ValueError("published enum removed: " + name)
        new_entries = current["enums"][name]
        if any(entry not in new_entries for entry in entries):
            raise ValueError("published enum value removed, reused or changed: " + name)
    for name, old_fields in previous["messages"].items():
        if name not in current["messages"]:
            raise ValueError("published message removed: " + name)
        new_fields = current["messages"][name]
        by_number = {field["number"]: field for field in new_fields}
        by_name = {field["name"]: field for field in new_fields}
        if len(by_number) != len(new_fields) or len(by_name) != len(new_fields):
            raise ValueError("duplicate field name/number: " + name)
        for field in old_fields:
            if by_number.get(field["number"]) != field or by_name.get(field["name"]) != field:
                raise ValueError("published field removed, reused or changed: " + name + "." + field["name"])
        if any(field["label"] == 2 for field in new_fields):
            raise ValueError("required field is not an additive proto3 evolution")

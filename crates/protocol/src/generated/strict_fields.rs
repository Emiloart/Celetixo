// @generated from proto/descriptor.bin; never edit field metadata by hand.
const ESCALATION_POLICY: &[Field] = &[(1, Text, false), (2, Uint32, false)];
const IMPACT_POLICY: &[Field] = &[(1, Uint32, false), (2, Text, true), (3, Text, true)];
const PROVENANCE: &[Field] = &[
    (1, Text, false),
    (2, Text, false),
    (3, Text, false),
    (4, Text, false),
];
const RESOURCE_BUDGET: &[Field] = &[(1, Uint64, false), (2, Uint64, false), (3, Uint64, false)];
const RETRY_POLICY: &[Field] = &[(1, Uint32, false), (2, Bool, false)];
const ROLLBACK_POLICY: &[Field] = &[(1, Bool, false), (2, Bool, false)];
const TASK_INTENT: &[Field] = &[
    (1, Text, false),
    (2, Text, false),
    (3, Text, false),
    (4, Text, false),
    (5, Text, false),
    (6, Text, true),
    (7, Text, true),
    (8, Text, true),
    (9, Text, true),
    (10, Nested(IMPACT_POLICY), false),
    (11, Nested(VERIFICATION_REQUIREMENTS), false),
    (12, Nested(RESOURCE_BUDGET), false),
    (13, Nested(RETRY_POLICY), false),
    (14, Nested(ROLLBACK_POLICY), false),
    (15, Nested(ESCALATION_POLICY), false),
    (16, Nested(PROVENANCE), false),
];
const VERIFICATION_REQUIREMENTS: &[Field] = &[
    (1, Uint32, false),
    (2, Text, true),
    (3, Bool, false),
    (4, Bool, false),
];
const SCOPE_SELECTOR: &[Field] = &[(1, Text, false), (2, Text, false), (3, Text, false)];
const TASK_INTENT_ENVELOPE: &[Field] = &[
    (1, Uint32, false),
    (2, Nested(SCOPE_SELECTOR), false),
    (3, Nested(TASK_INTENT), false),
    (4, Text, false),
    (5, Text, false),
    (6, Text, true),
];
const ENVELOPE: &[Field] = TASK_INTENT_ENVELOPE;

# Mixed-language repository benchmark fixture v1

`template/` is self-authored source, not downloaded project data. The benchmark
materializes `__UNIT__` as `unit000`, `unit001`, and so on in a disposable Git
repository. Each unit includes TypeScript, JavaScript, Python, Go and Rust source
with a base function, direct caller, transitive test consumer and unrelated
function. Five root manifests/configurations participate in the index. Rust's
`src/lib.rs` is generated with one module declaration per unit.

`reference.json` contains manually specified **file-level potential impact** for
changing the base declaration in `unit000`. The reference includes the changed
file. It follows the authored call relationships; it is read separately from the
index, and no analyzer output generates or updates these labels. A change to the
fixture or these judgments requires reviewing the labels and incrementing the
fixture/reference version if their meaning changes.

Python's `dynamic.py` deliberately calls `getattr(math_ops, "base")(2)`. That file
belongs in the reference impact even when the structural resolver cannot connect
the string-valued attribute to the declaration. Its resulting false negative is
reported rather than removed from the denominator. This is a small exploratory
reference suite, not a statistically representative quality claim or an unseen
evaluation corpus. Do not use it as training data or describe it as a confirmatory
comparison of coding agents.

The harness does not run these source files or their dependency lifecycle hooks.
It checks repeat capture identity and exact incremental/rebuild equivalence, and
reports all capture/index uncertainty. Recognizing these files does not claim
complete ecosystem support or complete semantic impact.

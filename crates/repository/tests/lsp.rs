//! Protocol peers below are trusted fixtures. They do not claim language semantics.
use celetixo_engine::ids::{Digest, OrganizationId, RepositoryId, Scope, SnapshotId, WorkspaceId};
use celetixo_repository::lsp::*;
use serde_json::{Value, json};
use std::collections::VecDeque;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const URI: &str = "file:///immutable/src/main.rs";
fn config(snapshot: &str) -> SessionConfig {
    SessionConfig::new(
        Scope {
            organization: OrganizationId::new("org").unwrap(),
            workspace: WorkspaceId::new("workspace").unwrap(),
            repository: RepositoryId::new("repo").unwrap(),
        },
        SnapshotId(Digest::of("fixture", snapshot.as_bytes())),
        "file:///immutable".into(),
        "trusted-fixture".into(),
        "1".into(),
        Value::Null,
        json!({"rust-analyzer":{"cargo":{"allTargets":false}}}),
    )
    .unwrap()
}
#[derive(Default)]
struct Wire {
    sent: Vec<Value>,
    received: VecDeque<Value>,
}
#[derive(Clone)]
struct FixtureTransport(Arc<Mutex<Wire>>);
impl MessageTransport for FixtureTransport {
    fn send(&mut self, message: &Value) -> Result<(), LspError> {
        self.0.lock().unwrap().sent.push(message.clone());
        Ok(())
    }
    fn receive(&mut self, _timeout: Duration) -> Result<Option<Value>, LspError> {
        Ok(self.0.lock().unwrap().received.pop_front())
    }
}
fn new_session() -> (LspSession, Arc<Mutex<Wire>>) {
    let wire = Arc::new(Mutex::new(Wire::default()));
    wire.lock().unwrap().received.push_back(json!({"jsonrpc":"2.0","id":1,"result":{"capabilities":{"textDocumentSync":{"openClose":true,"change":1},"definitionProvider":true,"referencesProvider":true,"typeDefinitionProvider":true,"implementationProvider":true,"callHierarchyProvider":true}}}));
    let mut session = LspSession::new(config("one"), Box::new(FixtureTransport(wire.clone())));
    session.initialize(Duration::from_secs(2)).unwrap();
    session
        .open_document(
            URI.into(),
            "rust",
            7,
            "fn target() {}\nfn caller() { target(); }\n".into(),
        )
        .unwrap();
    (session, wire)
}
fn location(line: u32, character: u32) -> Value {
    json!({"uri":URI,"range":{"start":{"line":line,"character":character},"end":{"line":line,"character":character+6}}})
}
fn call_item(line: u32, character: u32, name: &str) -> Value {
    let mut item = location(line, character);
    item["selectionRange"] = item["range"].clone();
    item["name"] = json!(name);
    item["kind"] = json!(12);
    item
}
fn response(wire: &Arc<Mutex<Wire>>, ticket: &RequestTicket, result: Value) {
    wire.lock()
        .unwrap()
        .received
        .push_back(json!({"jsonrpc":"2.0","id":ticket.id(),"result":result}));
}

#[test]
fn content_length_framing_is_byte_exact_and_survives_partial_payloads() {
    let message = json!({"jsonrpc":"2.0","method":"example","params":"café 🦀"});
    let frame = encode_message(&message).unwrap();
    let mut partial = frame[..frame.len() - 3].to_vec();
    assert_eq!(decode_message(&mut partial).unwrap(), None);
    partial.extend_from_slice(&frame[frame.len() - 3..]);
    partial.extend_from_slice(
        &encode_message(&json!({"jsonrpc":"2.0","id":9,"result":null})).unwrap(),
    );
    assert_eq!(decode_message(&mut partial).unwrap(), Some(message));
    assert_eq!(decode_message(&mut partial).unwrap().unwrap()["id"], 9);
    assert!(partial.is_empty());
    assert!(
        decode_message(&mut b"Content-Length: 1\r\nContent-Length: 1\r\n\r\n0".to_vec()).is_err()
    );
    assert!(decode_message(&mut b"Content-Length: 999999999\r\n\r\n".to_vec()).is_err());
    assert!(decode_message(&mut b"Unknown: 3\r\n\r\n{}".to_vec()).is_err());
}
#[test]
fn utf16_positions_preserve_multibyte_text_and_reject_surrogate_splits() {
    let text = "é🦀x\r\n变量()\n";
    assert_eq!(
        utf16_to_byte(
            text,
            TextPosition {
                line: 0,
                character: 3
            }
        )
        .unwrap(),
        6
    );
    assert!(
        utf16_to_byte(
            text,
            TextPosition {
                line: 0,
                character: 2
            }
        )
        .is_err()
    );
    assert_eq!(
        utf16_to_byte(
            text,
            TextPosition {
                line: 1,
                character: 2
            }
        )
        .unwrap(),
        15
    );
    assert_eq!(
        byte_to_utf16(text, 15).unwrap(),
        TextPosition {
            line: 1,
            character: 2
        }
    );
    assert!(byte_to_utf16(text, 3).is_err());
    assert!(
        utf16_to_byte(
            text,
            TextPosition {
                line: 2,
                character: 1
            }
        )
        .is_err()
    );
    assert!(
        utf16_to_byte(
            text,
            TextPosition {
                line: 3,
                character: 0
            }
        )
        .is_err()
    );
}
#[test]
fn reordered_queries_keep_exact_request_snapshot_document_and_operation_binding() {
    let (mut session, wire) = new_session();
    let first = session
        .start_query(
            SemanticQuery::Definition,
            URI,
            TextPosition {
                line: 1,
                character: 14,
            },
            Duration::from_secs(2),
        )
        .unwrap();
    let second = session
        .start_query(
            SemanticQuery::References,
            URI,
            TextPosition {
                line: 0,
                character: 3,
            },
            Duration::from_secs(2),
        )
        .unwrap();
    response(&wire, &second, json!([location(1, 14)]));
    response(&wire, &first, location(0, 3));
    let first_result = session.wait(&first).unwrap();
    let second_result = session.wait(&second).unwrap();
    assert_eq!(first_result.query, SemanticQuery::Definition);
    assert_eq!(second_result.query, SemanticQuery::References);
    assert_eq!(first_result.binding.document_version, 7);
    assert_eq!(first_result.binding.snapshot, config("one").key.snapshot);
    assert_eq!(first_result.completeness, SemanticCompleteness::QueryOnly);
    assert!(
        first_result
            .validate_snapshot(
                &config("two").key.scope,
                config("two").key.snapshot,
                first_result.binding.server_generation
            )
            .is_err()
    );
    assert_eq!(
        session
            .resolve_location(&first_result.locations[0])
            .unwrap(),
        (3, 9)
    );
}
#[test]
fn cancelled_closed_and_timed_out_requests_cannot_publish_delayed_results() {
    let (mut session, wire) = new_session();
    let ticket = session
        .start_query(
            SemanticQuery::Definition,
            URI,
            TextPosition {
                line: 0,
                character: 3,
            },
            Duration::from_secs(1),
        )
        .unwrap();
    session.cancel(&ticket).unwrap();
    response(&wire, &ticket, location(0, 3));
    session.poll(Duration::ZERO).unwrap();
    assert!(matches!(
        session.take_result(&ticket).unwrap(),
        Err(LspError::Cancelled(_))
    ));
    assert!(matches!(
        &session.take_events()[0],
        LspEvent::RejectedResponse { .. }
    ));
    let timeout = session
        .start_query(
            SemanticQuery::Definition,
            URI,
            TextPosition {
                line: 0,
                character: 3,
            },
            Duration::ZERO,
        )
        .unwrap();
    response(&wire, &timeout, location(0, 3));
    session.poll(Duration::ZERO).unwrap();
    assert!(matches!(
        session.take_result(&timeout).unwrap(),
        Err(LspError::Timeout(_))
    ));
    let closing = session
        .start_query(
            SemanticQuery::Definition,
            URI,
            TextPosition {
                line: 0,
                character: 3,
            },
            Duration::from_secs(1),
        )
        .unwrap();
    session.close_document(URI).unwrap();
    assert!(matches!(
        session.take_result(&closing).unwrap(),
        Err(LspError::Cancelled(_))
    ));
    assert!(
        session
            .open_document(URI.into(), "rust", 8, "new content".into())
            .is_err()
    );
}
#[test]
fn unsupported_server_errors_and_incomplete_answers_are_explicit() {
    let (mut session, wire) = new_session();
    let ticket = session
        .start_query(
            SemanticQuery::Definition,
            URI,
            TextPosition {
                line: 0,
                character: 3,
            },
            Duration::from_secs(2),
        )
        .unwrap();
    wire.lock().unwrap().received.push_back(json!({"jsonrpc":"2.0","id":ticket.id(),"error":{"code":-32801,"message":"Content modified"}}));
    assert!(matches!(
        session.wait(&ticket),
        Err(LspError::Server { code: -32801, .. })
    ));
    let ticket = session
        .start_query(
            SemanticQuery::References,
            URI,
            TextPosition {
                line: 0,
                character: 3,
            },
            Duration::from_secs(2),
        )
        .unwrap();
    response(&wire, &ticket, json!({"isIncomplete":true,"items":[]}));
    assert!(session.wait(&ticket).is_err());
    let wire = Arc::new(Mutex::new(Wire::default()));
    wire.lock()
        .unwrap()
        .received
        .push_back(json!({"jsonrpc":"2.0","id":1,"result":{"capabilities":{"textDocumentSync":{"openClose":true,"change":1}}}}));
    let mut session = LspSession::new(config("one"), Box::new(FixtureTransport(wire)));
    session.initialize(Duration::from_secs(1)).unwrap();
    session
        .open_document(URI.into(), "rust", 1, "fn x() {}".into())
        .unwrap();
    assert!(matches!(
        session.start_query(
            SemanticQuery::Definition,
            URI,
            TextPosition {
                line: 0,
                character: 3
            },
            Duration::from_secs(1)
        ),
        Err(LspError::Unsupported(_))
    ));
}
#[test]
fn diagnostics_require_document_versions_and_unapproved_server_mutations_are_denied() {
    let (mut session, wire) = new_session();
    for version in [Value::Null, json!(6), json!(7)] {
        wire.lock().unwrap().received.push_back(json!({"jsonrpc":"2.0","method":"textDocument/publishDiagnostics","params":{"uri":URI,"version":version,"diagnostics":[]}}));
        session.poll(Duration::ZERO).unwrap();
    }
    assert_eq!(session.diagnostics()[URI].binding.document_version, 7);
    assert_eq!(
        session
            .take_events()
            .iter()
            .filter(|e| matches!(e, LspEvent::RejectedDiagnostics { .. }))
            .count(),
        2
    );
    wire.lock().unwrap().received.push_back(json!({"jsonrpc":"2.0","id":"server-1","method":"workspace/applyEdit","params":{"edit":{}}}));
    session.poll(Duration::ZERO).unwrap();
    assert_eq!(
        wire.lock().unwrap().sent.last().unwrap()["error"]["code"],
        -32601
    );
}
#[test]
fn call_hierarchy_opaque_continuations_remain_session_bound() {
    let (mut session, wire) = new_session();
    let ticket = session
        .start_query(
            SemanticQuery::PrepareCallHierarchy,
            URI,
            TextPosition {
                line: 0,
                character: 3,
            },
            Duration::from_secs(2),
        )
        .unwrap();
    let mut item = location(0, 3);
    item["name"] = json!("target");
    item["kind"] = json!(12);
    item["selectionRange"] = item["range"].clone();
    item["data"] = json!({"opaque":"handle"});
    response(&wire, &ticket, json!([item]));
    let result = session.wait(&ticket).unwrap();
    let incoming = session
        .start_call_query(
            SemanticQuery::IncomingCalls,
            &result.call_items[0],
            Duration::from_secs(2),
        )
        .unwrap();
    response(
        &wire,
        &incoming,
        json!([{"from":call_item(1,3,"caller"),"fromRanges":[]} ]),
    );
    assert_eq!(
        session.wait(&incoming).unwrap().locations[0]
            .range
            .start
            .line,
        1
    );
    let (mut foreign, _) = new_session();
    assert!(
        foreign
            .start_call_query(
                SemanticQuery::IncomingCalls,
                &result.call_items[0],
                Duration::from_secs(1)
            )
            .is_err()
    );
    foreign.close_document(URI).unwrap();
    assert!(
        foreign
            .start_call_query(
                SemanticQuery::IncomingCalls,
                &result.call_items[0],
                Duration::from_secs(1)
            )
            .is_err()
    );
}
#[test]
fn shared_pool_reuses_one_connection_and_isolates_scope_snapshot_and_configuration() {
    let pool = SharedLanguageServices::default();
    let connect = || {
        Ok(
            Box::new(FixtureTransport(Arc::new(Mutex::new(Wire::default()))))
                as Box<dyn MessageTransport>,
        )
    };
    let a = pool.get_or_connect(config("one"), connect).unwrap();
    let b = pool
        .get_or_connect(config("one"), || panic!("duplicate server connection"))
        .unwrap();
    assert!(Arc::ptr_eq(&a, &b));
    let different_snapshot = pool.get_or_connect(config("two"), connect).unwrap();
    assert!(!Arc::ptr_eq(&a, &different_snapshot));
    let mut organization = config("one");
    organization.key.scope.organization = OrganizationId::new("other").unwrap();
    assert!(!Arc::ptr_eq(
        &a,
        &pool.get_or_connect(organization, connect).unwrap()
    ));
    let changed = SessionConfig::new(
        config("one").key.scope,
        config("one").key.snapshot,
        "file:///immutable".into(),
        "fixture".into(),
        "2".into(),
        Value::Null,
        json!({}),
    )
    .unwrap();
    assert!(!Arc::ptr_eq(
        &a,
        &pool.get_or_connect(changed, connect).unwrap()
    ));
    assert_eq!(pool.len().unwrap(), 4);
}
#[test]
fn adapters_accept_initial_language_ids_without_claiming_language_completeness() {
    let (mut session, _) = new_session();
    for (n, language) in [
        "typescript",
        "typescriptreact",
        "javascript",
        "javascriptreact",
        "python",
        "go",
        "rust",
    ]
    .iter()
    .enumerate()
    {
        session
            .open_document(format!("file:///immutable/file{n}"), language, 1, "".into())
            .unwrap();
    }
    assert!(
        session
            .open_document(
                "file:///immutable/../elsewhere.rs".into(),
                "rust",
                1,
                "".into()
            )
            .is_err()
    );
    assert!(
        session
            .open_document(
                "file:///immutable/%2E%2E/elsewhere.rs".into(),
                "rust",
                1,
                "".into()
            )
            .is_err()
    );
    assert!(
        session
            .open_document(
                "file:///immutable-other/file.rs".into(),
                "rust",
                1,
                "".into()
            )
            .is_err()
    );
}
#[test]
fn live_tcp_peer_uses_content_length_framing() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let peer = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut bytes = Vec::new();
        let mut chunk = [0; 4096];
        loop {
            let count = stream.read(&mut chunk).unwrap();
            assert!(count > 0);
            bytes.extend_from_slice(&chunk[..count]);
            if let Some(message) = decode_message(&mut bytes).unwrap() {
                assert_eq!(message["method"], "initialize");
                stream.write_all(&encode_message(&json!({"jsonrpc":"2.0","id":message["id"],"result":{"capabilities":{"textDocumentSync":{"openClose":true,"change":1}}}})).unwrap()).unwrap();
                break;
            }
        }
        // Read the initialized notification before closing to avoid a reset race.
        loop {
            if decode_message(&mut bytes).unwrap().is_some() {
                break;
            }
            let count = stream.read(&mut chunk).unwrap();
            assert!(count > 0);
            bytes.extend_from_slice(&chunk[..count]);
        }
    });
    let connection =
        TcpTransport::new(TcpStream::connect(address).unwrap(), Duration::from_secs(2)).unwrap();
    let mut session = LspSession::new(config("tcp"), Box::new(connection));
    session.initialize(Duration::from_secs(2)).unwrap();
    peer.join().unwrap();
}

#[test]
fn semantic_facts_supplement_structural_index_without_erasing_coverage_uncertainty() {
    use celetixo_engine::ids::Revision;
    use celetixo_engine::snapshot::*;
    use celetixo_repository::index::*;
    use std::collections::BTreeMap;
    let text = "fn target() {}\nfn caller() { target(); }\n";
    let snapshot = Snapshot::new(
        config("one").key.scope,
        Revision(0),
        SnapshotManifest {
            base_commit: "trusted-fixture".into(),
            canonicalization_version: 1,
            inclusion_policy: InclusionPolicy::complete(),
            provenance: "trusted immutable test input".into(),
            inputs: vec![WorkspaceInput {
                path: "src/main.rs".into(),
                layer: InputLayer::WorkingTree,
                kind: InputKind::File,
                mode: FileMode::Regular,
                bytes: Some(text.as_bytes().to_vec()),
            }],
            environment: BTreeMap::new(),
        },
        BTreeMap::new(),
    )
    .unwrap();
    let mut index = Indexer::new(IndexConfig::default())
        .unwrap()
        .build(&snapshot)
        .unwrap();
    let wire = Arc::new(Mutex::new(Wire::default()));
    wire.lock().unwrap().received.push_back(
        json!({"jsonrpc":"2.0","id":1,"result":{"capabilities":{"textDocumentSync":{"openClose":true,"change":1},"definitionProvider":true,"referencesProvider":true}}}),
    );
    let cfg = SessionConfig::new(
        snapshot.scope().clone(),
        snapshot.id(),
        "file:///immutable".into(),
        "trusted".into(),
        "1".into(),
        Value::Null,
        Value::Null,
    )
    .unwrap();
    let mut server = LspSession::new(cfg, Box::new(FixtureTransport(wire.clone())));
    server.initialize(Duration::from_secs(1)).unwrap();
    server
        .open_document(URI.into(), "rust", 1, text.into())
        .unwrap();
    let ticket = server
        .start_query(
            SemanticQuery::Definition,
            URI,
            TextPosition {
                line: 1,
                character: 15,
            },
            Duration::from_secs(2),
        )
        .unwrap();
    response(&wire, &ticket, location(0, 3));
    let result = server.wait(&ticket).unwrap();
    let references_ticket = server
        .start_query(
            SemanticQuery::References,
            URI,
            TextPosition {
                line: 1,
                character: 15,
            },
            Duration::from_secs(2),
        )
        .unwrap();
    response(&wire, &references_ticket, json!([location(1, 14)]));
    let references_result = server.wait(&references_ticket).unwrap();
    let before = index.clone();
    assert!(
        server
            .supplement_index(&mut index, &references_result)
            .is_err()
    );
    assert_eq!(index, before);
    let old_generation = index.generation;
    let report = server.supplement_index(&mut index, &result).unwrap();
    assert_eq!(report.added_edges, 1);
    assert_eq!(report.unmapped_locations, 0);
    let references_report = server
        .supplement_index(&mut index, &references_result)
        .unwrap();
    assert_eq!(references_report.added_edges, 1);
    assert_ne!(index.generation, old_generation);
    assert!(
        index
            .edges
            .iter()
            .any(|edge| edge.provenance.resolution == Resolution::Analyzer
                && edge.kind == EdgeKind::Call)
    );
    assert!(
        index
            .issues
            .iter()
            .any(|issue| issue.kind == IssueKind::SemanticAnalysisUnavailable)
    );
    assert!(!index.is_complete());
    index.validate(&snapshot).unwrap();
    let mut forged = result.clone();
    forged.binding.document_version = 2;
    let before = index.clone();
    assert!(server.supplement_index(&mut index, &forged).is_err());
    assert_eq!(index, before);
}

/// Opt-in integration with the installed, approved rust-analyzer component. All
/// analyzed source is generated below; cargo/build scripts/proc macros are off.
#[test]
#[ignore = "requires an explicitly installed rust-analyzer component; run with --ignored"]
fn real_rust_analyzer_definition_and_references() {
    use celetixo_engine::ids::Revision;
    use celetixo_engine::snapshot::*;
    use celetixo_repository::index::{EdgeKind, IndexConfig, Indexer, Resolution};
    use std::collections::BTreeMap;
    use std::process::{Command, Stdio};
    use std::sync::mpsc;
    use std::time::Instant;
    // Notifications can make poll return immediately. A bounded elapsed-time
    // loading window must not collapse into a fixed number of rapid empty
    // responses while rust-analyzer loads the linked project asynchronously.
    fn poll_for(session: &mut LspSession, duration: Duration) {
        let deadline = Instant::now() + duration;
        while let Some(remaining) = deadline.checked_duration_since(Instant::now()) {
            if remaining.is_zero() {
                break;
            }
            session.poll(remaining).unwrap();
        }
    }
    struct ChildGuard(std::process::Child);
    impl Drop for ChildGuard {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    struct StdioPeer {
        stdin: std::process::ChildStdin,
        rx: mpsc::Receiver<Result<Value, LspError>>,
    }
    impl MessageTransport for StdioPeer {
        fn send(&mut self, message: &Value) -> Result<(), LspError> {
            self.stdin.write_all(&encode_message(message)?)?;
            self.stdin.flush()?;
            Ok(())
        }
        fn receive(&mut self, timeout: Duration) -> Result<Option<Value>, LspError> {
            match self.rx.recv_timeout(timeout) {
                Ok(value) => value.map(Some),
                Err(mpsc::RecvTimeoutError::Timeout) => Ok(None),
                Err(error) => Err(LspError::Protocol(error.to_string())),
            }
        }
    }
    let executable = Command::new("rustup")
        .args(["which", "rust-analyzer"])
        .output()
        .expect("rustup must be installed");
    assert!(
        executable.status.success(),
        "install rust-analyzer via rustup first"
    );
    let executable = String::from_utf8(executable.stdout).unwrap();
    let fixture = tempfile::tempdir().unwrap();
    let path = fixture.path().join("main.rs");
    let text = "fn target() {}\nfn caller() { target(); }\n";
    std::fs::write(&path, text).unwrap();
    let snapshot = Snapshot::new(
        config("real").key.scope,
        Revision(0),
        SnapshotManifest {
            base_commit: "trusted-rust-analyzer-fixture".into(),
            canonicalization_version: 1,
            inclusion_policy: InclusionPolicy::complete(),
            provenance: "trusted materialized Rust fixture".into(),
            inputs: vec![WorkspaceInput {
                path: "main.rs".into(),
                layer: InputLayer::WorkingTree,
                kind: InputKind::File,
                mode: FileMode::Regular,
                bytes: Some(text.as_bytes().to_vec()),
            }],
            environment: BTreeMap::new(),
        },
        BTreeMap::new(),
    )
    .unwrap();
    let mut index = Indexer::new(IndexConfig::default())
        .unwrap()
        .build(&snapshot)
        .unwrap();
    let mut child = ChildGuard(
        Command::new(executable.trim())
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .current_dir(fixture.path())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let stdin = child.0.stdin.take().unwrap();
    let mut stdout = child.0.stdout.take().unwrap();
    let (tx, rx) = mpsc::channel();
    let reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let mut chunk = [0; 8192];
        loop {
            match decode_message(&mut bytes) {
                Ok(Some(value)) => {
                    if tx.send(Ok(value)).is_err() {
                        break;
                    }
                    continue;
                }
                Ok(None) => {}
                Err(error) => {
                    let _ = tx.send(Err(error));
                    break;
                }
            }
            match stdout.read(&mut chunk) {
                Ok(0) => break,
                Ok(count) => bytes.extend_from_slice(&chunk[..count]),
                Err(error) => {
                    let _ = tx.send(Err(error.into()));
                    break;
                }
            }
        }
    });
    let root = format!("file://{}", fixture.path().display());
    let uri = format!("file://{}", path.display());
    let cfg=SessionConfig::new(snapshot.scope().clone(),snapshot.id(),root,"rust-analyzer".into(),"1.94.1".into(),json!({"linkedProjects":[{"crates":[{"root_module":path,"edition":"2021","deps":[],"cfg":[]}]}],"cargo":{"buildScripts":{"enable":false},"autoreload":false},"procMacro":{"enable":false},"checkOnSave":false}),json!({})).unwrap();
    let mut session = LspSession::new(cfg, Box::new(StdioPeer { stdin, rx }));
    session.initialize(Duration::from_secs(20)).unwrap();
    session
        .open_document(uri.clone(), "rust", 1, text.into())
        .unwrap();
    let mut definition = None;
    let definition_deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < definition_deadline {
        let ticket = session
            .start_query(
                SemanticQuery::Definition,
                &uri,
                TextPosition {
                    line: 1,
                    character: 15,
                },
                definition_deadline
                    .saturating_duration_since(Instant::now())
                    .min(Duration::from_secs(10)),
            )
            .unwrap();
        match session.wait(&ticket) {
            Ok(result) if !result.locations.is_empty() => {
                definition = Some(result);
                break;
            }
            Ok(_) | Err(LspError::Server { code: -32801, .. }) => {
                poll_for(
                    &mut session,
                    definition_deadline
                        .saturating_duration_since(Instant::now())
                        .min(Duration::from_millis(100)),
                );
            }
            Err(error) => panic!("real language server failed: {error}"),
        }
    }
    let definition = definition.expect("rust-analyzer must resolve trusted fixture target");
    assert_eq!(definition.locations[0].uri, uri);
    assert_eq!(
        definition.locations[0].range.start,
        TextPosition {
            line: 0,
            character: 3
        }
    );
    assert_eq!(
        session
            .supplement_index(&mut index, &definition)
            .unwrap()
            .added_edges,
        1
    );
    let mut references = None;
    let references_deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < references_deadline {
        let ticket = session
            .start_query(
                SemanticQuery::References,
                &uri,
                TextPosition {
                    line: 0,
                    character: 3,
                },
                references_deadline
                    .saturating_duration_since(Instant::now())
                    .min(Duration::from_secs(10)),
            )
            .unwrap();
        match session.wait(&ticket) {
            Ok(result)
                if result
                    .locations
                    .iter()
                    .any(|location| location.range.start.line == 1) =>
            {
                references = Some(result);
                break;
            }
            Ok(_) | Err(LspError::Server { code: -32801, .. }) => {
                poll_for(
                    &mut session,
                    references_deadline
                        .saturating_duration_since(Instant::now())
                        .min(Duration::from_millis(100)),
                );
            }
            Err(error) => panic!("real language server reference query failed: {error}"),
        }
    }
    let references = references.expect("rust-analyzer must identify the trusted fixture caller");
    assert_eq!(
        session
            .supplement_index(&mut index, &references)
            .unwrap()
            .added_edges,
        1
    );
    assert!(index.edges.iter().any(|edge| edge.kind == EdgeKind::Call
        && edge.provenance.resolution == Resolution::Analyzer
        && index.nodes[&edge.from].name == "caller"
        && index.nodes[&edge.to].name == "target"));
    index.validate(&snapshot).unwrap();
    assert!(!index.is_complete());
    drop(session);
    drop(child);
    reader.join().unwrap();
}

#[test]
fn type_definition_implementation_and_outgoing_calls_negotiate_distinct_operations() {
    let (mut session, wire) = new_session();
    for query in [SemanticQuery::TypeDefinition, SemanticQuery::Implementation] {
        let ticket = session
            .start_query(
                query,
                URI,
                TextPosition {
                    line: 0,
                    character: 3,
                },
                Duration::from_secs(2),
            )
            .unwrap();
        response(&wire, &ticket, json!([location(0, 3)]));
        let result = session.wait(&ticket).unwrap();
        assert_eq!(result.query, query);
        assert_eq!(result.locations.len(), 1);
    }
    let prepare = session
        .start_query(
            SemanticQuery::PrepareCallHierarchy,
            URI,
            TextPosition {
                line: 1,
                character: 3,
            },
            Duration::from_secs(2),
        )
        .unwrap();
    let mut item = location(1, 3);
    item["selectionRange"] = item["range"].clone();
    item["name"] = json!("caller");
    item["kind"] = json!(12);
    response(&wire, &prepare, json!([item]));
    let result = session.wait(&prepare).unwrap();
    let outgoing = session
        .start_call_query(
            SemanticQuery::OutgoingCalls,
            &result.call_items[0],
            Duration::from_secs(2),
        )
        .unwrap();
    response(
        &wire,
        &outgoing,
        json!([{"to":call_item(0,3,"target"),"fromRanges":[]} ]),
    );
    assert_eq!(
        session.wait(&outgoing).unwrap().locations[0]
            .range
            .start
            .line,
        0
    );
}
#[test]
fn configuration_key_tampering_and_servers_without_buffer_sync_are_rejected() {
    let pool = SharedLanguageServices::default();
    let mut cfg = config("one");
    cfg.settings = json!({"changed":true});
    assert!(
        pool.get_or_connect(cfg, || panic!("tampered configuration cannot connect"))
            .is_err()
    );
    let wire = Arc::new(Mutex::new(Wire::default()));
    wire.lock().unwrap().received.push_back(
        json!({"jsonrpc":"2.0","id":1,"result":{"capabilities":{"definitionProvider":true}}}),
    );
    let mut session = LspSession::new(config("one"), Box::new(FixtureTransport(wire)));
    session.initialize(Duration::from_secs(1)).unwrap();
    assert!(matches!(
        session.open_document(URI.into(), "rust", 1, "fn x() {}".into()),
        Err(LspError::Unsupported(_))
    ));
}

#[test]
fn external_target_documents_are_explicitly_unbound_to_the_snapshot() {
    let (mut session, wire) = new_session();
    let ticket = session
        .start_query(
            SemanticQuery::Definition,
            URI,
            TextPosition {
                line: 0,
                character: 3,
            },
            Duration::from_secs(1),
        )
        .unwrap();
    let mut external = location(0, 3);
    external["uri"] = json!("file:///external/dependency.rs");
    response(&wire, &ticket, external);
    let result = session.wait(&ticket).unwrap();
    assert!(result.location_bindings.is_empty());
    assert!(
        result
            .unbound_location_uris
            .contains("file:///external/dependency.rs")
    );
    assert_eq!(result.completeness, SemanticCompleteness::QueryOnly);
}

#[test]
fn tcp_partial_frame_timeout_recovers_without_losing_payload_boundaries() {
    use std::sync::mpsc;
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let (ready_tx, ready_rx) = mpsc::channel();
    let (continue_tx, continue_rx) = mpsc::channel();
    let message = json!({"jsonrpc":"2.0","id":5,"result":"🦀 partial payload"});
    let expected = message.clone();
    let peer = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let bytes = encode_message(&message).unwrap();
        let split = bytes.len() - 5;
        stream.write_all(&bytes[..split]).unwrap();
        stream.flush().unwrap();
        ready_tx.send(()).unwrap();
        continue_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        stream.write_all(&bytes[split..]).unwrap();
    });
    let mut transport =
        TcpTransport::new(TcpStream::connect(address).unwrap(), Duration::from_secs(1)).unwrap();
    ready_rx.recv_timeout(Duration::from_secs(1)).unwrap();
    assert!(
        transport
            .receive(Duration::from_millis(10))
            .unwrap()
            .is_none()
    );
    continue_tx.send(()).unwrap();
    assert_eq!(
        transport.receive(Duration::from_secs(1)).unwrap(),
        Some(expected)
    );
    peer.join().unwrap();
}

#[test]
fn colliding_request_numbers_cannot_cross_language_server_connections() {
    let (mut first, first_wire) = new_session();
    let (mut second, second_wire) = new_session();
    let a = first
        .start_query(
            SemanticQuery::Definition,
            URI,
            TextPosition {
                line: 0,
                character: 3,
            },
            Duration::from_secs(1),
        )
        .unwrap();
    let b = second
        .start_query(
            SemanticQuery::References,
            URI,
            TextPosition {
                line: 0,
                character: 3,
            },
            Duration::from_secs(1),
        )
        .unwrap();
    assert_eq!(a.id(), b.id());
    assert!(matches!(
        second.take_result(&a),
        Some(Err(LspError::StaleResult))
    ));
    assert!(matches!(second.cancel(&a), Err(LspError::StaleResult)));
    assert!(matches!(second.wait(&a), Err(LspError::StaleResult)));
    response(&first_wire, &a, location(0, 3));
    response(&second_wire, &b, json!([location(1, 14)]));
    assert_eq!(first.wait(&a).unwrap().query, SemanticQuery::Definition);
    assert_eq!(second.wait(&b).unwrap().query, SemanticQuery::References);
}

fn indexed_fixture(
    extra: Vec<(&str, &str)>,
) -> (
    LspSession,
    Arc<Mutex<Wire>>,
    celetixo_repository::index::SourceIndex,
) {
    use celetixo_engine::ids::Revision;
    use celetixo_engine::snapshot::*;
    use celetixo_repository::index::*;
    use std::collections::BTreeMap;
    let text = "fn target() {}\nfn caller() { target(); }\n";
    let mut inputs = vec![WorkspaceInput {
        path: "src/main.rs".into(),
        layer: InputLayer::WorkingTree,
        kind: InputKind::File,
        mode: FileMode::Regular,
        bytes: Some(text.as_bytes().to_vec()),
    }];
    inputs.extend(extra.into_iter().map(|(path, text)| WorkspaceInput {
        path: path.into(),
        layer: InputLayer::WorkingTree,
        kind: InputKind::File,
        mode: FileMode::Regular,
        bytes: Some(text.as_bytes().to_vec()),
    }));
    let snapshot = Snapshot::new(
        config("one").key.scope,
        Revision(0),
        SnapshotManifest {
            base_commit: "trusted-fixture".into(),
            canonicalization_version: 1,
            inclusion_policy: InclusionPolicy::complete(),
            provenance: "trusted immutable test input".into(),
            inputs,
            environment: BTreeMap::new(),
        },
        BTreeMap::new(),
    )
    .unwrap();
    let index = Indexer::new(IndexConfig::default())
        .unwrap()
        .build(&snapshot)
        .unwrap();
    let wire = Arc::new(Mutex::new(Wire::default()));
    wire.lock().unwrap().received.push_back(json!({"jsonrpc":"2.0","id":1,"result":{"capabilities":{"textDocumentSync":{"openClose":true,"change":1},"definitionProvider":true,"referencesProvider":true,"callHierarchyProvider":true}}}));
    let cfg = SessionConfig::new(
        snapshot.scope().clone(),
        snapshot.id(),
        "file:///immutable".into(),
        "trusted".into(),
        "1".into(),
        Value::Null,
        Value::Null,
    )
    .unwrap();
    let mut server = LspSession::new(cfg, Box::new(FixtureTransport(wire.clone())));
    server.initialize(Duration::from_secs(1)).unwrap();
    server
        .open_document(URI.into(), "rust", 1, text.into())
        .unwrap();
    (server, wire, index)
}
#[test]
fn modified_or_foreign_semantic_results_cannot_claim_analyzer_provenance() {
    let (mut server, wire, mut index) = indexed_fixture(vec![]);
    let ticket = server
        .start_query(
            SemanticQuery::Definition,
            URI,
            TextPosition {
                line: 1,
                character: 15,
            },
            Duration::from_secs(1),
        )
        .unwrap();
    response(&wire, &ticket, location(0, 3));
    let result = server.wait(&ticket).unwrap();
    let before = index.clone();
    for variant in 0..4 {
        let mut modified = result.clone();
        match variant {
            0 => modified.locations[0].range.start.character = 4,
            1 => modified.query = SemanticQuery::TypeDefinition,
            2 => modified.position.character = 3,
            _ => {
                modified
                    .unbound_location_uris
                    .insert("file:///foreign.rs".into());
            }
        };
        assert!(server.supplement_index(&mut index, &modified).is_err());
        assert_eq!(index, before);
    }
    let (foreign, _, _) = indexed_fixture(vec![]);
    assert!(foreign.supplement_index(&mut index, &result).is_err());
    assert_eq!(index, before);
    assert_eq!(
        server
            .supplement_index(&mut index, &result)
            .unwrap()
            .added_edges,
        1
    );
}
#[test]
fn unbound_response_targets_require_a_fresh_query_after_document_synchronization() {
    let late_uri = "file:///immutable/late.rs";
    let late_text = "fn target() {}\n";
    let (mut server, wire, mut index) = indexed_fixture(vec![("late.rs", late_text)]);
    let ticket = server
        .start_query(
            SemanticQuery::Definition,
            URI,
            TextPosition {
                line: 1,
                character: 15,
            },
            Duration::from_secs(1),
        )
        .unwrap();
    let mut target = location(0, 3);
    target["uri"] = json!(late_uri);
    response(&wire, &ticket, target.clone());
    let result = server.wait(&ticket).unwrap();
    assert!(result.unbound_location_uris.contains(late_uri));
    server
        .open_document(late_uri.into(), "rust", 1, late_text.into())
        .unwrap();
    let report = server.supplement_index(&mut index, &result).unwrap();
    assert_eq!(report.added_edges, 0);
    assert_eq!(report.unmapped_locations, 1);
    let fresh = server
        .start_query(
            SemanticQuery::Definition,
            URI,
            TextPosition {
                line: 1,
                character: 15,
            },
            Duration::from_secs(1),
        )
        .unwrap();
    response(&wire, &fresh, target);
    let fresh = server.wait(&fresh).unwrap();
    assert_eq!(
        server
            .supplement_index(&mut index, &fresh)
            .unwrap()
            .added_edges,
        1
    );
}
#[test]
fn unsupported_definition_locations_are_not_mapped_to_an_enclosing_declaration() {
    let (mut server, wire, mut index) = indexed_fixture(vec![]);
    let ticket = server
        .start_query(
            SemanticQuery::Definition,
            URI,
            TextPosition {
                line: 1,
                character: 15,
            },
            Duration::from_secs(1),
        )
        .unwrap();
    // An occurrence lies inside caller's full declaration span but is not a
    // declaration name. It cannot become an authoritative caller definition.
    response(&wire, &ticket, location(1, 14));
    let result = server.wait(&ticket).unwrap();
    let report = server.supplement_index(&mut index, &result).unwrap();
    assert_eq!(report.added_edges, 0);
    assert_eq!(report.unmapped_locations, 1);
}
#[test]
fn malformed_correlated_responses_have_a_terminal_error_and_do_not_stall_other_queries() {
    let (mut server, wire) = new_session();
    let bad = server
        .start_query(
            SemanticQuery::Definition,
            URI,
            TextPosition {
                line: 0,
                character: 3,
            },
            Duration::from_secs(1),
        )
        .unwrap();
    let good = server
        .start_query(
            SemanticQuery::References,
            URI,
            TextPosition {
                line: 0,
                character: 3,
            },
            Duration::from_secs(1),
        )
        .unwrap();
    let mut invalid = location(0, 3);
    invalid["range"]["end"]["character"] = json!(999);
    response(&wire, &bad, invalid);
    response(&wire, &good, json!([location(1, 14)]));
    assert!(matches!(server.wait(&bad), Err(LspError::InvalidPosition)));
    assert_eq!(server.wait(&good).unwrap().query, SemanticQuery::References);
}
#[test]
fn fatal_tcp_framing_failure_cannot_be_retried_as_a_valid_connection() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let peer = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .write_all(b"Content-Length: 1\r\nContent-Length: 1\r\n\r\n0")
            .unwrap();
    });
    let mut transport =
        TcpTransport::new(TcpStream::connect(address).unwrap(), Duration::from_secs(1)).unwrap();
    assert!(matches!(
        transport.receive(Duration::from_secs(1)),
        Err(LspError::Protocol(_))
    ));
    assert!(matches!(
        transport.receive(Duration::ZERO),
        Err(LspError::TransportUnusable)
    ));
    assert!(matches!(
        transport.send(&json!({"jsonrpc":"2.0","id":1,"method":"initialize"})),
        Err(LspError::TransportUnusable)
    ));
    peer.join().unwrap();
}
#[test]
fn malformed_headers_json_rpc_shapes_and_uris_fail_explicitly() {
    for bytes in [
        b"Content-Length: +2\r\n\r\n{}".as_slice(),
        b"Content-Type: application/vscode-jsonrpc; charset=utf-16\r\nContent-Length: 2\r\n\r\n{}"
            .as_slice(),
        b"Content-L\xc3\xa9ngth: 2\r\n\r\n{}".as_slice(),
    ] {
        assert!(decode_message(&mut bytes.to_vec()).is_err());
    }
    let (mut server, wire) = new_session();
    wire.lock()
        .unwrap()
        .received
        .push_back(json!({"jsonrpc":"2.0","id":100,"method":false,"result":null}));
    assert!(server.poll(Duration::ZERO).is_err());
    for uri in [
        "file:///immutable/raw space.rs",
        "file:///immutable/bad%GG.rs",
        "file:///immutable/bad%.rs",
    ] {
        assert!(
            server
                .open_document(uri.into(), "rust", 1, "".into())
                .is_err()
        );
    }
}

#[test]
fn hierarchy_queries_add_exact_caller_to_callee_edges() {
    use celetixo_repository::index::{EdgeKind, Resolution};
    let (mut server, wire, mut index) = indexed_fixture(vec![]);
    let prepare = server
        .start_query(
            SemanticQuery::PrepareCallHierarchy,
            URI,
            TextPosition {
                line: 1,
                character: 3,
            },
            Duration::from_secs(1),
        )
        .unwrap();
    response(&wire, &prepare, json!([call_item(1, 3, "caller")]));
    let prepared = server.wait(&prepare).unwrap();
    let outgoing = server
        .start_call_query(
            SemanticQuery::OutgoingCalls,
            &prepared.call_items[0],
            Duration::from_secs(1),
        )
        .unwrap();
    response(
        &wire,
        &outgoing,
        json!([{"to":call_item(0,3,"target"),"fromRanges":[location(1,14)["range"]]}]),
    );
    let outgoing = server.wait(&outgoing).unwrap();
    assert_eq!(
        server
            .supplement_index(&mut index, &outgoing)
            .unwrap()
            .added_edges,
        1
    );
    let target = &outgoing.call_items[0];
    let incoming = server
        .start_call_query(SemanticQuery::IncomingCalls, target, Duration::from_secs(1))
        .unwrap();
    response(
        &wire,
        &incoming,
        json!([{"from":call_item(1,3,"caller"),"fromRanges":[location(1,14)["range"]]}]),
    );
    let incoming = server.wait(&incoming).unwrap();
    assert_eq!(
        server
            .supplement_index(&mut index, &incoming)
            .unwrap()
            .added_edges,
        1
    );
    assert!(index.edges.iter().any(|edge| edge.kind == EdgeKind::Call
        && edge.provenance.resolution == Resolution::Analyzer
        && index.nodes[&edge.from].name == "caller"
        && index.nodes[&edge.to].name == "target"));
    index.validate_generation().unwrap();
    assert!(!index.is_complete());
}
#[test]
fn unbound_hierarchy_tokens_cannot_be_upgraded_by_later_document_opens() {
    let late_uri = "file:///immutable/late.rs";
    let (mut server, wire) = new_session();
    let ticket = server
        .start_query(
            SemanticQuery::PrepareCallHierarchy,
            URI,
            TextPosition {
                line: 0,
                character: 3,
            },
            Duration::from_secs(1),
        )
        .unwrap();
    let mut item = call_item(0, 3, "target");
    item["uri"] = json!(late_uri);
    response(&wire, &ticket, json!([item]));
    let result = server.wait(&ticket).unwrap();
    server
        .open_document(late_uri.into(), "rust", 1, "fn target() {}\n".into())
        .unwrap();
    assert!(matches!(
        server.start_call_query(
            SemanticQuery::IncomingCalls,
            &result.call_items[0],
            Duration::from_secs(1)
        ),
        Err(LspError::StaleResult)
    ));
}
#[test]
fn reference_queries_in_function_bodies_require_an_actual_referent_identity() {
    let (mut server, wire, mut index) = indexed_fixture(vec![]);
    let before = index.clone();
    let ticket = server
        .start_query(
            SemanticQuery::References,
            URI,
            TextPosition {
                line: 1,
                character: 12,
            },
            Duration::from_secs(1),
        )
        .unwrap();
    response(&wire, &ticket, json!([location(0, 3)]));
    let result = server.wait(&ticket).unwrap();
    assert!(matches!(
        server.supplement_index(&mut index, &result),
        Err(LspError::Unsupported(_))
    ));
    assert_eq!(index, before);
}
#[test]
fn pending_and_event_budgets_apply_backpressure_and_recover_when_consumed() {
    let (mut server, wire) = new_session();
    let mut tickets = Vec::new();
    for _ in 0..4096 {
        tickets.push(
            server
                .start_query(
                    SemanticQuery::Definition,
                    URI,
                    TextPosition {
                        line: 0,
                        character: 3,
                    },
                    Duration::from_secs(60),
                )
                .unwrap(),
        );
    }
    assert!(matches!(
        server.start_query(
            SemanticQuery::Definition,
            URI,
            TextPosition {
                line: 0,
                character: 3
            },
            Duration::from_secs(60)
        ),
        Err(LspError::Capacity)
    ));
    server.cancel(&tickets[0]).unwrap();
    assert!(matches!(
        server.take_result(&tickets[0]),
        Some(Err(LspError::Cancelled(_)))
    ));
    server
        .start_query(
            SemanticQuery::Definition,
            URI,
            TextPosition {
                line: 0,
                character: 3,
            },
            Duration::from_secs(60),
        )
        .unwrap();
    let (mut events, event_wire) = new_session();
    let method = "notification".repeat(70_000);
    for _ in 0..5 {
        event_wire
            .lock()
            .unwrap()
            .received
            .push_back(json!({"jsonrpc":"2.0","method":method}));
    }
    let mut capacity = false;
    for _ in 0..5 {
        if matches!(events.poll(Duration::ZERO), Err(LspError::Capacity)) {
            capacity = true;
            break;
        }
    }
    assert!(capacity);
    assert!(!events.take_events().is_empty());
    event_wire.lock().unwrap().received.clear();
    event_wire
        .lock()
        .unwrap()
        .received
        .push_back(json!({"jsonrpc":"2.0","method":"window/logMessage"}));
    events.poll(Duration::ZERO).unwrap();
    assert_eq!(events.take_events().len(), 1);
    assert!(
        wire.lock()
            .unwrap()
            .sent
            .iter()
            .any(|message| message["method"] == "$/cancelRequest")
    );
}

#[test]
fn language_server_document_language_must_match_the_indexed_source_adapter() {
    let (_, _, mut index) = indexed_fixture(vec![]);
    let before = index.clone();
    let wire = Arc::new(Mutex::new(Wire::default()));
    wire.lock().unwrap().received.push_back(json!({"jsonrpc":"2.0","id":1,"result":{"capabilities":{"textDocumentSync":{"openClose":true,"change":1},"definitionProvider":true}}}));
    let cfg = SessionConfig::new(
        index.scope.clone(),
        index.snapshot,
        "file:///immutable".into(),
        "trusted".into(),
        "1".into(),
        Value::Null,
        Value::Null,
    )
    .unwrap();
    let mut server = LspSession::new(cfg, Box::new(FixtureTransport(wire.clone())));
    server.initialize(Duration::from_secs(1)).unwrap();
    server
        .open_document(
            URI.into(),
            "python",
            1,
            "fn target() {}\nfn caller() { target(); }\n".into(),
        )
        .unwrap();
    let ticket = server
        .start_query(
            SemanticQuery::Definition,
            URI,
            TextPosition {
                line: 1,
                character: 15,
            },
            Duration::from_secs(1),
        )
        .unwrap();
    response(&wire, &ticket, location(0, 3));
    let result = server.wait(&ticket).unwrap();
    assert_eq!(result.binding.language_id, "python");
    assert!(matches!(
        server.supplement_index(&mut index, &result),
        Err(LspError::Unsupported(_))
    ));
    assert_eq!(index, before);
}

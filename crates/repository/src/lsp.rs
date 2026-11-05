//! Shared, version-bound LSP queries over caller-established transports.
//!
//! This module never starts repository-selected programs. The caller must establish a
//! trusted, isolated language server against an immutable materialization of `snapshot`.
//! LSP answers supplement a structural index; even a successful query does not prove
//! complete repository impact. LSP 3.17 locations use UTF-16 code units.
use celetixo_engine::ids::{Digest, Scope, SnapshotId};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use thiserror::Error;

const MAX_FRAME: usize = 16 * 1024 * 1024;
const MAX_HEADER: usize = 8192;
const MAX_PENDING: usize = 4096;
const MAX_DOCUMENT_BYTES: usize = 128 * 1024 * 1024;
const MAX_EVENT_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug, Error)]
pub enum LspError {
    #[error("language-server transport: {0}")]
    Io(#[from] io::Error),
    #[error("invalid LSP protocol: {0}")]
    Protocol(String),
    #[error("language server is not initialized")]
    NotInitialized,
    #[error("language-server operation unsupported: {0}")]
    Unsupported(String),
    #[error("document is not open in this immutable session: {0}")]
    UnknownDocument(String),
    #[error("snapshot document cannot be changed in place: {0}")]
    DocumentChanged(String),
    #[error("invalid UTF-16 source coordinate")]
    InvalidPosition,
    #[error("language-server request {0} timed out")]
    Timeout(u64),
    #[error("language-server request {0} was cancelled")]
    Cancelled(u64),
    #[error("server error {code}: {message}")]
    Server { code: i64, message: String },
    #[error("stale or foreign semantic result")]
    StaleResult,
    #[error("language-service pool is poisoned")]
    Poisoned,
    #[error("language-service resource budget exhausted")]
    Capacity,
    #[error("language-server transport is unusable after a fatal framing or I/O error")]
    TransportUnusable,
}

/// Transport implementations must bound reads by `timeout` and writes by their
/// configured transport deadline. They exchange JSON payloads, not unframed lines.
pub trait MessageTransport: Send {
    fn send(&mut self, message: &Value) -> Result<(), LspError>;
    fn receive(&mut self, timeout: Duration) -> Result<Option<Value>, LspError>;
}

/// Content-Length framed TCP transport. Its receive buffer survives partial-frame
/// timeouts, so retrying a timed-out read cannot reinterpret payload as headers.
pub struct TcpTransport {
    stream: TcpStream,
    buffered: Vec<u8>,
    write_timeout: Duration,
    failed: bool,
}
impl TcpTransport {
    pub fn new(stream: TcpStream, write_timeout: Duration) -> Result<Self, LspError> {
        if write_timeout.is_zero() {
            return Err(LspError::Protocol("zero write timeout".into()));
        }
        stream.set_write_timeout(Some(write_timeout))?;
        Ok(Self {
            stream,
            buffered: Vec::new(),
            write_timeout,
            failed: false,
        })
    }
}
impl MessageTransport for TcpTransport {
    fn send(&mut self, message: &Value) -> Result<(), LspError> {
        if self.failed {
            return Err(LspError::TransportUnusable);
        }
        let bytes = encode_message(message)?;
        let finish = deadline(self.write_timeout)?;
        let outcome = (|| -> Result<(), LspError> {
            let mut written = 0;
            while written < bytes.len() {
                let remaining = finish
                    .checked_duration_since(Instant::now())
                    .filter(|remaining| !remaining.is_zero())
                    .ok_or_else(|| {
                        io::Error::new(io::ErrorKind::TimedOut, "LSP write deadline exceeded")
                    })?;
                self.stream.set_write_timeout(Some(remaining))?;
                match self.stream.write(&bytes[written..]) {
                    Ok(0) => {
                        return Err(io::Error::new(
                            io::ErrorKind::WriteZero,
                            "LSP write made no progress",
                        )
                        .into());
                    }
                    Ok(count) => written += count,
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                    Err(error) => return Err(error.into()),
                }
            }
            self.stream.flush()?;
            Ok(())
        })();
        if outcome.is_err() {
            self.failed = true;
        }
        outcome?;
        Ok(())
    }
    fn receive(&mut self, timeout: Duration) -> Result<Option<Value>, LspError> {
        if self.failed {
            return Err(LspError::TransportUnusable);
        }
        let deadline = Instant::now()
            .checked_add(timeout)
            .ok_or_else(|| LspError::Protocol("timeout overflow".into()))?;
        loop {
            match decode_message(&mut self.buffered) {
                Ok(Some(message)) => return Ok(Some(message)),
                Ok(None) => {}
                Err(error) => {
                    self.failed = true;
                    return Err(error);
                }
            }
            let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
                return Ok(None);
            };
            if remaining.is_zero() {
                return Ok(None);
            }
            if let Err(error) = self.stream.set_read_timeout(Some(remaining)) {
                self.failed = true;
                return Err(error.into());
            }
            let mut chunk = [0; 8192];
            match self.stream.read(&mut chunk) {
                Ok(0) => {
                    self.failed = true;
                    return Err(LspError::Io(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "language server closed stream",
                    )));
                }
                Ok(count) => {
                    self.buffered.extend_from_slice(&chunk[..count]);
                    if self.buffered.len() > MAX_FRAME + MAX_HEADER + chunk.len() {
                        self.failed = true;
                        return Err(LspError::Protocol("frame exceeds limit".into()));
                    }
                }
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                    ) =>
                {
                    return Ok(None);
                }
                Err(error) => {
                    self.failed = true;
                    return Err(error.into());
                }
            }
        }
    }
}

/// Encode one standard LSP message, with an explicit bounded payload.
pub fn encode_message(message: &Value) -> Result<Vec<u8>, LspError> {
    let payload =
        serde_json::to_vec(message).map_err(|error| LspError::Protocol(error.to_string()))?;
    if payload.len() > MAX_FRAME {
        return Err(LspError::Protocol("frame exceeds limit".into()));
    }
    let mut bytes = format!("Content-Length: {}\r\n\r\n", payload.len()).into_bytes();
    bytes.extend_from_slice(&payload);
    Ok(bytes)
}

/// Decode a complete frame if available. Duplicate/missing lengths, invalid JSON
/// and excessive headers/payloads are errors; no bytes are discarded on timeout.
pub fn decode_message(bytes: &mut Vec<u8>) -> Result<Option<Value>, LspError> {
    let Some(header_end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") else {
        if bytes.len() > MAX_HEADER {
            return Err(LspError::Protocol("header exceeds limit".into()));
        }
        return Ok(None);
    };
    if header_end > MAX_HEADER {
        return Err(LspError::Protocol("header exceeds limit".into()));
    }
    let header = std::str::from_utf8(&bytes[..header_end])
        .map_err(|_| LspError::Protocol("non-ASCII header".into()))?;
    if !header.is_ascii() {
        return Err(LspError::Protocol("non-ASCII header".into()));
    }
    let mut length = None;
    for line in header.split("\r\n") {
        let (name, value) = line
            .split_once(':')
            .ok_or_else(|| LspError::Protocol("malformed header".into()))?;
        if name.eq_ignore_ascii_case("Content-Length") {
            if length.is_some() {
                return Err(LspError::Protocol("duplicate Content-Length".into()));
            }
            let value = value.trim();
            if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
                return Err(LspError::Protocol("invalid Content-Length".into()));
            }
            length = Some(
                value
                    .parse::<usize>()
                    .map_err(|_| LspError::Protocol("invalid Content-Length".into()))?,
            );
        } else if name.eq_ignore_ascii_case("Content-Type") {
            for part in value.split(';').skip(1) {
                if let Some((key, encoding)) = part.trim().split_once('=')
                    && key.eq_ignore_ascii_case("charset")
                    && !matches!(encoding.to_ascii_lowercase().as_str(), "utf-8" | "utf8")
                {
                    return Err(LspError::Unsupported("non UTF-8 LSP frame".into()));
                }
            }
        }
    }
    let length = length.ok_or_else(|| LspError::Protocol("missing Content-Length".into()))?;
    if length > MAX_FRAME {
        return Err(LspError::Protocol("frame exceeds limit".into()));
    }
    let end = header_end + 4 + length;
    if bytes.len() < end {
        return Ok(None);
    }
    let value = serde_json::from_slice(&bytes[header_end + 4..end])
        .map_err(|error| LspError::Protocol(error.to_string()))?;
    bytes.drain(..end);
    Ok(Some(value))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct TextPosition {
    pub line: u32,
    pub character: u32,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct TextRange {
    pub start: TextPosition,
    pub end: TextPosition,
}
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct SemanticLocation {
    pub uri: String,
    pub range: TextRange,
}

/// Converts an LSP UTF-16 position to a byte offset without splitting a scalar.
pub fn utf16_to_byte(text: &str, position: TextPosition) -> Result<usize, LspError> {
    let mut start = 0;
    for _ in 0..position.line {
        let Some(newline) = text[start..].find('\n') else {
            return Err(LspError::InvalidPosition);
        };
        start += newline + 1;
    }
    let line = text[start..].split('\n').next().unwrap_or("");
    let line = line.strip_suffix('\r').unwrap_or(line);
    let mut units = 0;
    for (offset, scalar) in line.char_indices() {
        if units == position.character {
            return Ok(start + offset);
        }
        units += scalar.len_utf16() as u32;
        if units > position.character {
            return Err(LspError::InvalidPosition);
        }
    }
    if units == position.character {
        Ok(start + line.len())
    } else {
        Err(LspError::InvalidPosition)
    }
}

pub fn byte_to_utf16(text: &str, offset: usize) -> Result<TextPosition, LspError> {
    if offset > text.len() || !text.is_char_boundary(offset) {
        return Err(LspError::InvalidPosition);
    }
    let prefix = &text[..offset];
    let line = prefix.bytes().filter(|byte| *byte == b'\n').count();
    let start = prefix.rfind('\n').map_or(0, |at| at + 1);
    if text.as_bytes().get(offset) == Some(&b'\n') && prefix.ends_with('\r') {
        return Err(LspError::InvalidPosition);
    }
    Ok(TextPosition {
        line: u32::try_from(line).map_err(|_| LspError::InvalidPosition)?,
        character: u32::try_from(prefix[start..].encode_utf16().count())
            .map_err(|_| LspError::InvalidPosition)?,
    })
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct SessionKey {
    pub scope: Scope,
    pub snapshot: SnapshotId,
    /// Includes supplied server adapter identity/version and immutable root URI,
    /// initialization options and analyzer configuration; used for pool isolation.
    pub server_generation: Digest,
}

#[derive(Debug, Clone)]
pub struct SessionConfig {
    pub key: SessionKey,
    pub root_uri: String,
    pub server_name: String,
    pub server_version: String,
    pub initialization_options: Value,
    /// Approved settings only. Server requests cannot inspect host configuration.
    pub settings: Value,
}
impl SessionConfig {
    fn validate(&self) -> Result<(), LspError> {
        let expected = Self::new(
            self.key.scope.clone(),
            self.key.snapshot,
            self.root_uri.clone(),
            self.server_name.clone(),
            self.server_version.clone(),
            self.initialization_options.clone(),
            self.settings.clone(),
        )?;
        if expected.key != self.key {
            return Err(LspError::StaleResult);
        }
        Ok(())
    }
    /// Construct a generation from every supplied analyzer/materialization input.
    pub fn new(
        scope: Scope,
        snapshot: SnapshotId,
        root_uri: String,
        server_name: String,
        server_version: String,
        initialization_options: Value,
        settings: Value,
    ) -> Result<Self, LspError> {
        validate_file_uri(&root_uri)?;
        if server_name.trim().is_empty() || server_version.trim().is_empty() {
            return Err(LspError::Protocol(
                "server identity/version required".into(),
            ));
        }
        let bytes = serde_json::to_vec(&json!({"root":root_uri,"server":server_name,"version":server_version,"initialization":initialization_options,"settings":settings})).map_err(|error| LspError::Protocol(error.to_string()))?;
        let server_generation = Digest::of("celetixo/lsp-generation/v1", &bytes);
        Ok(Self {
            key: SessionKey {
                scope,
                snapshot,
                server_generation,
            },
            root_uri,
            server_name,
            server_version,
            initialization_options,
            settings,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SemanticQuery {
    Definition,
    References,
    TypeDefinition,
    Implementation,
    PrepareCallHierarchy,
    IncomingCalls,
    OutgoingCalls,
}
impl SemanticQuery {
    fn method(self) -> &'static str {
        match self {
            Self::Definition => "textDocument/definition",
            Self::References => "textDocument/references",
            Self::TypeDefinition => "textDocument/typeDefinition",
            Self::Implementation => "textDocument/implementation",
            Self::PrepareCallHierarchy => "textDocument/prepareCallHierarchy",
            Self::IncomingCalls => "callHierarchy/incomingCalls",
            Self::OutgoingCalls => "callHierarchy/outgoingCalls",
        }
    }
    fn capability(self) -> &'static str {
        match self {
            Self::Definition => "definitionProvider",
            Self::References => "referencesProvider",
            Self::TypeDefinition => "typeDefinitionProvider",
            Self::Implementation => "implementationProvider",
            _ => "callHierarchyProvider",
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestBinding {
    pub scope: Scope,
    pub snapshot: SnapshotId,
    pub server_generation: Digest,
    pub document_uri: String,
    pub language_id: String,
    pub document_version: i32,
    pub document_digest: Digest,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SemanticCompleteness {
    /// The requested operation returned normally. This is *never* a repository
    /// completeness assertion; unresolved names and undiscovered queries remain.
    QueryOnly,
}
#[derive(Debug, Clone, PartialEq)]
pub struct SemanticResult {
    pub binding: RequestBinding,
    pub query: SemanticQuery,
    pub position: TextPosition,
    pub locations: Vec<SemanticLocation>,
    /// Target documents actually synchronized to this immutable session. Other
    /// targets (SDKs, dependencies, generated files) are explicitly unbound.
    pub location_bindings: BTreeMap<String, RequestBinding>,
    pub unbound_location_uris: BTreeSet<String>,
    /// Call hierarchy items are opaque, session-bound continuations, not trusted IDs.
    pub call_items: Vec<CallHierarchyItem>,
    pub completeness: SemanticCompleteness,
    // In-process integrity guard, not server authentication or a wire signature.
    origin: Arc<()>,
    seal: Digest,
    encoded_bytes: usize,
}
impl SemanticResult {
    fn contents_digest(&self) -> Digest {
        Digest::of("celetixo/lsp-result-seal/v1", &self.contents_bytes())
    }
    fn contents_bytes(&self) -> Vec<u8> {
        let bindings: Vec<_> = self
            .location_bindings
            .iter()
            .map(|(uri, binding)| json!({"uri":uri,"binding":binding_value(binding)}))
            .collect();
        let locations: Vec<_> = self.locations.iter().map(location_value).collect();
        let items:Vec<_>=self.call_items.iter().map(|item|json!({"source":binding_value(&item.binding),"target":item.target_binding.as_ref().map(binding_value),"item":item.raw})).collect();
        serde_json::to_vec(&json!({"binding":binding_value(&self.binding),"query":self.query.method(),"position":{"line":self.position.line,"character":self.position.character},"locations":locations,"bindings":bindings,"unbound":self.unbound_location_uris,"calls":items,"coverage":"query-only"})).expect("JSON values have a valid deterministic encoding")
    }
    pub fn validate_snapshot(
        &self,
        scope: &Scope,
        snapshot: SnapshotId,
        server_generation: Digest,
    ) -> Result<(), LspError> {
        if self.seal != self.contents_digest()
            || &self.binding.scope != scope
            || self.binding.snapshot != snapshot
            || self.binding.server_generation != server_generation
        {
            return Err(LspError::StaleResult);
        }
        Ok(())
    }
}
fn binding_value(binding: &RequestBinding) -> Value {
    json!({"scope":{"organization":binding.scope.organization.as_str(),"workspace":binding.scope.workspace.as_str(),"repository":binding.scope.repository.as_str()},"snapshot":binding.snapshot.to_string(),"server":binding.server_generation.to_string(),"uri":binding.document_uri,"language":binding.language_id,"version":binding.document_version,"document":binding.document_digest.to_string()})
}
fn location_value(location: &SemanticLocation) -> Value {
    json!({"uri":location.uri,"range":{"start":{"line":location.range.start.line,"character":location.range.start.character},"end":{"line":location.range.end.line,"character":location.range.end.character}}})
}
#[derive(Debug, Clone, PartialEq)]
pub struct CallHierarchyItem {
    binding: RequestBinding,
    raw: Value,
    origin: Arc<()>,
    target_binding: Option<RequestBinding>,
}
#[derive(Debug, Clone, PartialEq)]
pub struct BoundDiagnostics {
    pub binding: RequestBinding,
    pub diagnostics: Vec<Value>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LspEvent {
    RejectedResponse { request: String, reason: String },
    RejectedDiagnostics { uri: String, reason: String },
    ServerNotification { method: String },
    RejectedServerRequest { method: String },
}
#[derive(Debug, Clone)]
struct Document {
    text: String,
    version: i32,
    digest: Digest,
    language: String,
    open: bool,
}
#[derive(Debug, Clone)]
struct Pending {
    binding: RequestBinding,
    query: SemanticQuery,
    position: TextPosition,
    deadline: Instant,
}
#[derive(Debug, Clone)]
pub struct RequestTicket {
    id: u64,
    origin: Arc<()>,
}
impl RequestTicket {
    pub fn id(&self) -> u64 {
        self.id
    }
}

pub struct LspSession {
    config: SessionConfig,
    transport: Box<dyn MessageTransport>,
    initialized: bool,
    capabilities: Value,
    next_id: u64,
    documents: BTreeMap<String, Document>,
    pending: BTreeMap<u64, Pending>,
    completed: BTreeMap<u64, Result<SemanticResult, LspError>>,
    completed_sizes: BTreeMap<u64, usize>,
    completed_bytes: usize,
    events: VecDeque<LspEvent>,
    event_bytes: usize,
    diagnostics: BTreeMap<String, BoundDiagnostics>,
    marker: Arc<()>,
}
impl LspSession {
    pub fn new(config: SessionConfig, transport: Box<dyn MessageTransport>) -> Self {
        Self {
            config,
            transport,
            initialized: false,
            capabilities: Value::Null,
            next_id: 1,
            documents: BTreeMap::new(),
            pending: BTreeMap::new(),
            completed: BTreeMap::new(),
            completed_sizes: BTreeMap::new(),
            completed_bytes: 0,
            events: VecDeque::new(),
            event_bytes: 0,
            diagnostics: BTreeMap::new(),
            marker: Arc::new(()),
        }
    }
    pub fn key(&self) -> &SessionKey {
        &self.config.key
    }
    pub fn capabilities(&self) -> &Value {
        &self.capabilities
    }
    pub fn take_events(&mut self) -> Vec<LspEvent> {
        self.event_bytes = 0;
        self.events.drain(..).collect()
    }
    pub fn diagnostics(&self) -> &BTreeMap<String, BoundDiagnostics> {
        &self.diagnostics
    }
    pub fn initialize(&mut self, timeout: Duration) -> Result<(), LspError> {
        self.config.validate()?;
        if self.initialized {
            return Ok(());
        }
        let id = self.allocate_id()?;
        self.transport.send(&json!({"jsonrpc":"2.0","id":id,"method":"initialize","params":{"processId":null,"rootUri":self.config.root_uri,"workspaceFolders":[{"uri":self.config.root_uri,"name":"immutable-snapshot"}],"capabilities":{"general":{"positionEncodings":["utf-16"]},"textDocument":{"publishDiagnostics":{"versionSupport":true},"definition":{"linkSupport":true}},"workspace":{"configuration":true,"workspaceFolders":true}},"initializationOptions":self.config.initialization_options,"clientInfo":{"name":"celetixo","version":env!("CARGO_PKG_VERSION")}}}))?;
        let deadline = deadline(timeout)?;
        loop {
            let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
                return Err(LspError::Timeout(id));
            };
            let Some(message) = self.transport.receive(remaining)? else {
                return Err(LspError::Timeout(id));
            };
            validate_envelope(&message)?;
            if message.get("id").and_then(Value::as_u64) == Some(id)
                && message.get("method").is_none()
            {
                if let Some(error) = message.get("error") {
                    return Err(server_error(error)?);
                }
                let result = message
                    .get("result")
                    .ok_or_else(|| LspError::Protocol("initialize result missing".into()))?;
                self.capabilities = result
                    .get("capabilities")
                    .filter(|value| value.is_object())
                    .cloned()
                    .ok_or_else(|| LspError::Protocol("initialize capabilities missing".into()))?;
                if self
                    .capabilities
                    .get("positionEncoding")
                    .and_then(Value::as_str)
                    .is_some_and(|encoding| encoding != "utf-16")
                {
                    return Err(LspError::Unsupported(
                        "server selected non UTF-16 encoding".into(),
                    ));
                }
                self.transport
                    .send(&json!({"jsonrpc":"2.0","method":"initialized","params":{}}))?;
                self.initialized = true;
                return Ok(());
            }
            self.dispatch(message)?;
        }
    }
    /// Only immutable documents from the attested snapshot may be opened. A
    /// different content/version requires a different session/snapshot key.
    pub fn open_document(
        &mut self,
        uri: String,
        language: &str,
        version: i32,
        text: String,
    ) -> Result<(), LspError> {
        if !self.initialized {
            return Err(LspError::NotInitialized);
        }
        let synchronization = self.capabilities.get("textDocumentSync");
        if !synchronization.is_some_and(|value| {
            matches!(value.as_u64(), Some(1 | 2))
                || value.get("openClose") == Some(&Value::Bool(true))
        }) {
            return Err(LspError::Unsupported(
                "server cannot synchronize immutable document contents".into(),
            ));
        }
        validate_file_uri(&uri)?;
        if !uri_is_within(&uri, &self.config.root_uri) {
            return Err(LspError::Protocol("document outside snapshot root".into()));
        }
        if !matches!(
            language,
            "typescript"
                | "typescriptreact"
                | "javascript"
                | "javascriptreact"
                | "python"
                | "go"
                | "rust"
        ) {
            return Err(LspError::Unsupported(format!("language {language}")));
        }
        let digest = Digest::of("celetixo/lsp-document/v1", text.as_bytes());
        if !self.documents.contains_key(&uri)
            && (self.documents.len() >= MAX_PENDING
                || self
                    .documents
                    .values()
                    .map(|document| document.text.len())
                    .sum::<usize>()
                    .saturating_add(text.len())
                    > MAX_DOCUMENT_BYTES)
        {
            return Err(LspError::Capacity);
        }
        if let Some(previous) = self.documents.get(&uri) {
            if previous.digest != digest
                || previous.version != version
                || previous.language != language
            {
                return Err(LspError::DocumentChanged(uri));
            }
            if previous.open {
                return Ok(());
            }
        }
        self.transport.send(&json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{"uri":uri,"languageId":language,"version":version,"text":text}}}))?;
        self.documents.insert(
            uri,
            Document {
                text,
                version,
                digest,
                language: language.into(),
                open: true,
            },
        );
        Ok(())
    }
    pub fn close_document(&mut self, uri: &str) -> Result<(), LspError> {
        let document = self
            .documents
            .get(uri)
            .filter(|doc| doc.open)
            .ok_or_else(|| LspError::UnknownDocument(uri.into()))?;
        let _ = document;
        let tickets: Vec<_> = self
            .pending
            .iter()
            .filter(|(_, pending)| pending.binding.document_uri == uri)
            .map(|(id, _)| RequestTicket {
                id: *id,
                origin: Arc::clone(&self.marker),
            })
            .collect();
        for ticket in tickets {
            self.cancel(&ticket)?;
        }
        self.transport.send(&json!({"jsonrpc":"2.0","method":"textDocument/didClose","params":{"textDocument":{"uri":uri}}}))?;
        self.documents.get_mut(uri).expect("document checked").open = false;
        self.diagnostics.remove(uri);
        Ok(())
    }
    pub fn document_binding(&self, uri: &str) -> Result<RequestBinding, LspError> {
        let doc = self
            .documents
            .get(uri)
            .filter(|doc| doc.open)
            .ok_or_else(|| LspError::UnknownDocument(uri.into()))?;
        Ok(RequestBinding {
            scope: self.config.key.scope.clone(),
            snapshot: self.config.key.snapshot,
            server_generation: self.config.key.server_generation,
            document_uri: uri.into(),
            language_id: doc.language.clone(),
            document_version: doc.version,
            document_digest: doc.digest,
        })
    }
    pub fn resolve_location(
        &self,
        location: &SemanticLocation,
    ) -> Result<(usize, usize), LspError> {
        let doc = self
            .documents
            .get(&location.uri)
            .filter(|doc| doc.open)
            .ok_or_else(|| LspError::UnknownDocument(location.uri.clone()))?;
        let start = utf16_to_byte(&doc.text, location.range.start)?;
        let end = utf16_to_byte(&doc.text, location.range.end)?;
        if start > end {
            return Err(LspError::InvalidPosition);
        }
        Ok((start, end))
    }
    pub fn start_query(
        &mut self,
        query: SemanticQuery,
        uri: &str,
        position: TextPosition,
        timeout: Duration,
    ) -> Result<RequestTicket, LspError> {
        if matches!(
            query,
            SemanticQuery::IncomingCalls | SemanticQuery::OutgoingCalls
        ) {
            return Err(LspError::Protocol(
                "call query requires a bound hierarchy item".into(),
            ));
        }
        let binding = self.document_binding(uri)?;
        let doc = &self.documents[uri];
        utf16_to_byte(&doc.text, position)?;
        let mut params = json!({"textDocument":{"uri":uri},"position":{"line":position.line,"character":position.character}});
        if query == SemanticQuery::References {
            params["context"] = json!({"includeDeclaration":true});
        }
        self.submit(query, binding, position, params, timeout)
    }
    pub fn start_call_query(
        &mut self,
        query: SemanticQuery,
        item: &CallHierarchyItem,
        timeout: Duration,
    ) -> Result<RequestTicket, LspError> {
        if !Arc::ptr_eq(&item.origin, &self.marker) {
            return Err(LspError::StaleResult);
        }
        if !matches!(
            query,
            SemanticQuery::IncomingCalls | SemanticQuery::OutgoingCalls
        ) {
            return Err(LspError::Protocol(
                "expected incoming/outgoing call query".into(),
            ));
        }
        if self.document_binding(&item.binding.document_uri)? != item.binding {
            return Err(LspError::StaleResult);
        }
        let target_uri = item
            .raw
            .get("uri")
            .and_then(Value::as_str)
            .ok_or_else(|| LspError::Protocol("call item URI missing".into()))?;
        let binding = self.document_binding(target_uri)?;
        if item.target_binding.as_ref() != Some(&binding) {
            return Err(LspError::StaleResult);
        }
        let position = parse_range(&item.raw["selectionRange"])?.start;
        utf16_to_byte(&self.documents[target_uri].text, position)?;
        self.submit(query, binding, position, json!({"item":item.raw}), timeout)
    }
    fn submit(
        &mut self,
        query: SemanticQuery,
        binding: RequestBinding,
        position: TextPosition,
        params: Value,
        timeout: Duration,
    ) -> Result<RequestTicket, LspError> {
        if !self.initialized {
            return Err(LspError::NotInitialized);
        }
        if !self
            .capabilities
            .get(query.capability())
            .is_some_and(|value| value == &Value::Bool(true) || value.is_object())
        {
            return Err(LspError::Unsupported(query.method().into()));
        }
        if self.pending.len() + self.completed.len() >= MAX_PENDING {
            return Err(LspError::Capacity);
        }
        let deadline = deadline(timeout)?;
        let id = self.allocate_id()?;
        self.transport
            .send(&json!({"jsonrpc":"2.0","id":id,"method":query.method(),"params":params}))?;
        self.pending.insert(
            id,
            Pending {
                binding,
                query,
                position,
                deadline,
            },
        );
        Ok(RequestTicket {
            id,
            origin: Arc::clone(&self.marker),
        })
    }
    /// Multiplexed pump: responses for every outstanding request are dispatched by
    /// ID. A caller waiting for one ticket cannot misattribute a reordered answer.
    pub fn poll(&mut self, timeout: Duration) -> Result<(), LspError> {
        self.expire_requests()?;
        let wait = self
            .pending
            .values()
            .filter_map(|pending| pending.deadline.checked_duration_since(Instant::now()))
            .min()
            .map_or(timeout, |remaining| remaining.min(timeout));
        if let Some(message) = self.transport.receive(wait)? {
            self.dispatch(message)?;
        }
        self.expire_requests()?;
        Ok(())
    }
    pub fn take_result(
        &mut self,
        ticket: &RequestTicket,
    ) -> Option<Result<SemanticResult, LspError>> {
        if !Arc::ptr_eq(&ticket.origin, &self.marker) {
            return Some(Err(LspError::StaleResult));
        }
        let result = self.completed.remove(&ticket.id);
        self.completed_bytes -= self.completed_sizes.remove(&ticket.id).unwrap_or(0);
        result
    }
    pub fn wait(&mut self, ticket: &RequestTicket) -> Result<SemanticResult, LspError> {
        loop {
            if let Some(result) = self.take_result(ticket) {
                return result;
            }
            let pending = self.pending.get(&ticket.id).ok_or(LspError::StaleResult)?;
            let remaining = pending
                .deadline
                .checked_duration_since(Instant::now())
                .unwrap_or(Duration::ZERO);
            self.poll(remaining)?;
        }
    }
    pub fn cancel(&mut self, ticket: &RequestTicket) -> Result<(), LspError> {
        if !Arc::ptr_eq(&ticket.origin, &self.marker) {
            return Err(LspError::StaleResult);
        }
        if self.pending.remove(&ticket.id).is_none() {
            return Err(LspError::StaleResult);
        }
        self.complete(ticket.id, Err(LspError::Cancelled(ticket.id)));
        self.transport
            .send(&json!({"jsonrpc":"2.0","method":"$/cancelRequest","params":{"id":ticket.id}}))?;
        Ok(())
    }
    fn expire_requests(&mut self) -> Result<(), LspError> {
        let expired: Vec<_> = self
            .pending
            .iter()
            .filter(|(_, pending)| pending.deadline <= Instant::now())
            .map(|(id, _)| *id)
            .collect();
        for id in expired {
            self.pending.remove(&id);
            self.complete(id, Err(LspError::Timeout(id)));
            self.transport
                .send(&json!({"jsonrpc":"2.0","method":"$/cancelRequest","params":{"id":id}}))?;
        }
        Ok(())
    }
    fn allocate_id(&mut self) -> Result<u64, LspError> {
        let id = self.next_id;
        self.next_id = self.next_id.checked_add(1).ok_or(LspError::Capacity)?;
        Ok(id)
    }
    fn complete(&mut self, id: u64, mut outcome: Result<SemanticResult, LspError>) {
        self.completed_bytes -= self.completed_sizes.remove(&id).unwrap_or(0);
        let mut bytes = match &outcome {
            Ok(result) => result.encoded_bytes,
            Err(error) => error.to_string().len(),
        };
        if self.completed_bytes.saturating_add(bytes) > MAX_DOCUMENT_BYTES {
            outcome = Err(LspError::Capacity);
            bytes = 0;
        }
        self.completed_bytes += bytes;
        self.completed_sizes.insert(id, bytes);
        self.completed.insert(id, outcome);
    }
    fn event(&mut self, event: LspEvent) -> Result<(), LspError> {
        let bytes = match &event {
            LspEvent::RejectedResponse { request, reason } => {
                request.len().saturating_add(reason.len())
            }
            LspEvent::RejectedDiagnostics { uri, reason } => uri.len().saturating_add(reason.len()),
            LspEvent::ServerNotification { method }
            | LspEvent::RejectedServerRequest { method } => method.len(),
        };
        if self.events.len() >= MAX_PENDING
            || self.event_bytes.saturating_add(bytes) > MAX_EVENT_BYTES
        {
            return Err(LspError::Capacity);
        }
        self.event_bytes += bytes;
        self.events.push_back(event);
        Ok(())
    }
    fn dispatch(&mut self, message: Value) -> Result<(), LspError> {
        if self.events.len() >= MAX_PENDING {
            return Err(LspError::Capacity);
        }
        validate_envelope(&message)?;
        if let Some(method) = message.get("method").and_then(Value::as_str) {
            if let Some(id) = message.get("id") {
                return self.server_request(
                    id.clone(),
                    method,
                    message.get("params").unwrap_or(&Value::Null),
                );
            }
            if method == "textDocument/publishDiagnostics" {
                return self.accept_diagnostics(message.get("params").unwrap_or(&Value::Null));
            }
            self.event(LspEvent::ServerNotification {
                method: method.into(),
            })?;
            return Ok(());
        }
        let raw_id = message
            .get("id")
            .ok_or_else(|| LspError::Protocol("message has neither id nor method".into()))?;
        let Some(id) = raw_id.as_u64() else {
            self.event(LspEvent::RejectedResponse {
                request: raw_id.to_string(),
                reason: "foreign response identity".into(),
            })?;
            return Ok(());
        };
        let Some(pending) = self.pending.remove(&id) else {
            self.event(LspEvent::RejectedResponse {
                request: id.to_string(),
                reason: if id < self.next_id {
                    "late/duplicate response"
                } else {
                    "unknown request"
                }
                .into(),
            })?;
            return Ok(());
        };
        let result = (|| -> Result<SemanticResult, LspError> {
            if pending.deadline <= Instant::now() {
                return Err(LspError::Timeout(id));
            }
            if self.document_binding(&pending.binding.document_uri)? != pending.binding {
                return Err(LspError::StaleResult);
            }
            if let Some(error) = message.get("error") {
                return Err(server_error(error)?);
            }
            let mut result = parse_result(
                &pending,
                message
                    .get("result")
                    .ok_or_else(|| LspError::Protocol("response result missing".into()))?,
                &self.marker,
            )?;
            for location in &result.locations {
                match self.document_binding(&location.uri) {
                    Ok(binding) => {
                        self.resolve_location(location)?;
                        result
                            .location_bindings
                            .insert(location.uri.clone(), binding);
                    }
                    Err(_) => {
                        result.unbound_location_uris.insert(location.uri.clone());
                    }
                }
            }
            for item in &mut result.call_items {
                let uri = item
                    .raw
                    .get("uri")
                    .and_then(Value::as_str)
                    .ok_or_else(|| LspError::Protocol("call item URI missing".into()))?;
                item.target_binding = self.document_binding(uri).ok();
                if item.target_binding.is_some() {
                    self.resolve_location(&parse_location(&item.raw)?)?;
                }
            }
            let bytes = result.contents_bytes();
            result.encoded_bytes = bytes.len();
            result.seal = Digest::of("celetixo/lsp-result-seal/v1", &bytes);
            Ok(result)
        })();
        self.complete(id, result);
        Ok(())
    }
    fn accept_diagnostics(&mut self, params: &Value) -> Result<(), LspError> {
        let uri = params
            .get("uri")
            .and_then(Value::as_str)
            .ok_or_else(|| LspError::Protocol("diagnostics uri missing".into()))?;
        let binding = match self.document_binding(uri) {
            Ok(binding) => binding,
            Err(_) => {
                self.event(LspEvent::RejectedDiagnostics {
                    uri: uri.into(),
                    reason: "foreign/closed document".into(),
                })?;
                return Ok(());
            }
        };
        if params.get("version").and_then(Value::as_i64)
            != Some(i64::from(binding.document_version))
        {
            self.event(LspEvent::RejectedDiagnostics {
                uri: uri.into(),
                reason: "missing or stale document version".into(),
            })?;
            return Ok(());
        }
        let diagnostics = params
            .get("diagnostics")
            .and_then(Value::as_array)
            .ok_or_else(|| LspError::Protocol("diagnostics list missing".into()))?
            .clone();
        if serde_json::to_vec(
            &self
                .diagnostics
                .values()
                .map(|bound| &bound.diagnostics)
                .collect::<Vec<_>>(),
        )
        .map_err(|error| LspError::Protocol(error.to_string()))?
        .len()
        .saturating_add(
            serde_json::to_vec(&diagnostics)
                .map_err(|error| LspError::Protocol(error.to_string()))?
                .len(),
        ) > MAX_DOCUMENT_BYTES
        {
            return Err(LspError::Capacity);
        }
        for diagnostic in &diagnostics {
            let range = parse_range(
                diagnostic
                    .get("range")
                    .ok_or_else(|| LspError::Protocol("diagnostic range missing".into()))?,
            )?;
            self.resolve_location(&SemanticLocation {
                uri: uri.into(),
                range,
            })?;
        }
        self.diagnostics.insert(
            uri.into(),
            BoundDiagnostics {
                binding,
                diagnostics,
            },
        );
        Ok(())
    }
    fn server_request(&mut self, id: Value, method: &str, params: &Value) -> Result<(), LspError> {
        let result = match method {
            "workspace/configuration" => {
                let items = params
                    .get("items")
                    .and_then(Value::as_array)
                    .ok_or_else(|| LspError::Protocol("configuration items missing".into()))?;
                let values: Vec<_> = items
                    .iter()
                    .map(|item| {
                        item.get("section").and_then(Value::as_str).map_or_else(
                            || self.config.settings.clone(),
                            |section| {
                                section
                                    .split('.')
                                    .try_fold(&self.config.settings, |value, key| value.get(key))
                                    .cloned()
                                    .unwrap_or(Value::Null)
                            },
                        )
                    })
                    .collect();
                Some(Value::Array(values))
            }
            "workspace/workspaceFolders" => {
                Some(json!([{"uri":self.config.root_uri,"name":"immutable-snapshot"}]))
            }
            "window/workDoneProgress/create" => Some(Value::Null),
            _ => None,
        };
        if let Some(result) = result {
            self.transport
                .send(&json!({"jsonrpc":"2.0","id":id,"result":result}))?;
        } else {
            self.event(LspEvent::RejectedServerRequest {
                method: method.into(),
            })?;
            self.transport.send(&json!({"jsonrpc":"2.0","id":id,"error":{"code":-32601,"message":"not enabled by immutable Celetixo client"}}))?;
        }
        Ok(())
    }
}

fn deadline(timeout: Duration) -> Result<Instant, LspError> {
    Instant::now()
        .checked_add(timeout)
        .ok_or_else(|| LspError::Protocol("timeout overflow".into()))
}
fn validate_envelope(message: &Value) -> Result<(), LspError> {
    if !message.is_object() || message.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return Err(LspError::Protocol("expected JSON-RPC 2.0 object".into()));
    }
    if message.get("result").is_some() && message.get("error").is_some() {
        return Err(LspError::Protocol(
            "response contains result and error".into(),
        ));
    }
    if let Some(method) = message.get("method") {
        if !method.is_string() || message.get("result").is_some() || message.get("error").is_some()
        {
            return Err(LspError::Protocol(
                "malformed JSON-RPC method envelope".into(),
            ));
        }
    } else if message.get("id").is_none() {
        return Err(LspError::Protocol("response identity missing".into()));
    }
    Ok(())
}
fn server_error(error: &Value) -> Result<LspError, LspError> {
    Ok(LspError::Server {
        code: error
            .get("code")
            .and_then(Value::as_i64)
            .ok_or_else(|| LspError::Protocol("server error code missing".into()))?,
        message: error
            .get("message")
            .and_then(Value::as_str)
            .ok_or_else(|| LspError::Protocol("server error message missing".into()))?
            .into(),
    })
}
fn parse_position(value: &Value) -> Result<TextPosition, LspError> {
    let coordinate = |name: &str| {
        value
            .get(name)
            .and_then(Value::as_u64)
            .and_then(|number| u32::try_from(number).ok())
            .ok_or_else(|| LspError::Protocol(format!("invalid {name} coordinate")))
    };
    Ok(TextPosition {
        line: coordinate("line")?,
        character: coordinate("character")?,
    })
}
fn parse_range(value: &Value) -> Result<TextRange, LspError> {
    let range = TextRange {
        start: parse_position(&value["start"])?,
        end: parse_position(&value["end"])?,
    };
    if range.start > range.end {
        return Err(LspError::InvalidPosition);
    }
    Ok(range)
}
fn parse_location(value: &Value) -> Result<SemanticLocation, LspError> {
    let linked = value.get("targetUri").is_some();
    let uri = value
        .get(if linked { "targetUri" } else { "uri" })
        .and_then(Value::as_str)
        .ok_or_else(|| LspError::Protocol("location URI missing".into()))?;
    validate_file_uri(uri)?;
    let range = parse_range(
        &value[if linked {
            "targetSelectionRange"
        } else {
            "range"
        }],
    )?;
    Ok(SemanticLocation {
        uri: uri.into(),
        range,
    })
}
fn parse_call_location(value: &Value) -> Result<SemanticLocation, LspError> {
    let mut location = parse_location(value)?;
    let selection = parse_range(&value["selectionRange"])?;
    if selection.start < location.range.start
        || selection.end > location.range.end
        || value.get("name").and_then(Value::as_str).is_none()
        || value.get("kind").and_then(Value::as_u64).is_none()
    {
        return Err(LspError::Protocol("malformed call hierarchy item".into()));
    }
    location.range = selection;
    Ok(location)
}
fn parse_result(
    pending: &Pending,
    value: &Value,
    marker: &Arc<()>,
) -> Result<SemanticResult, LspError> {
    let values = if value.is_null() {
        Vec::new()
    } else if let Some(array) = value.as_array() {
        array.iter().collect()
    } else if value.is_object()
        && matches!(
            pending.query,
            SemanticQuery::Definition
                | SemanticQuery::TypeDefinition
                | SemanticQuery::Implementation
        )
        && !value.get("isIncomplete").is_some_and(|flag| flag == true)
    {
        vec![value]
    } else {
        return Err(LspError::Protocol(
            "incomplete or malformed semantic result".into(),
        ));
    };
    let mut locations = BTreeSet::new();
    let mut call_items = Vec::new();
    for item in values {
        match pending.query {
            SemanticQuery::PrepareCallHierarchy => {
                let location = parse_call_location(item)?;
                locations.insert(location);
                call_items.push(CallHierarchyItem {
                    binding: pending.binding.clone(),
                    raw: item.clone(),
                    origin: Arc::clone(marker),
                    target_binding: None,
                });
            }
            SemanticQuery::IncomingCalls | SemanticQuery::OutgoingCalls => {
                let key = if pending.query == SemanticQuery::IncomingCalls {
                    "from"
                } else {
                    "to"
                };
                let target = &item[key];
                locations.insert(parse_call_location(target)?);
                let ranges = item
                    .get("fromRanges")
                    .and_then(Value::as_array)
                    .ok_or_else(|| LspError::Protocol("call site ranges missing".into()))?;
                for range in ranges {
                    parse_range(range)?;
                }
                call_items.push(CallHierarchyItem {
                    binding: pending.binding.clone(),
                    raw: target.clone(),
                    origin: Arc::clone(marker),
                    target_binding: None,
                });
            }
            _ => {
                locations.insert(parse_location(item)?);
            }
        }
    }
    Ok(SemanticResult {
        binding: pending.binding.clone(),
        query: pending.query,
        position: pending.position,
        locations: locations.into_iter().collect(),
        location_bindings: BTreeMap::new(),
        unbound_location_uris: BTreeSet::new(),
        call_items,
        completeness: SemanticCompleteness::QueryOnly,
        origin: Arc::clone(marker),
        seal: Digest::of("celetixo/lsp-result-pending/v1", b""),
        encoded_bytes: 0,
    })
}
fn validate_file_uri(uri: &str) -> Result<(), LspError> {
    if !uri.starts_with("file:///")
        || !uri.is_ascii()
        || uri
            .chars()
            .any(|character| character.is_ascii_whitespace() || character.is_ascii_control())
        || uri.contains(['\0', '\r', '\n', '?', '#', '\\'])
        || uri.split('/').any(|part| part == ".." || part == ".")
    {
        return Err(LspError::Protocol(
            "expected normalized absolute file URI".into(),
        ));
    }
    let mut remaining = uri.as_bytes();
    while let Some(at) = remaining.iter().position(|byte| *byte == b'%') {
        let encoded = remaining
            .get(at + 1..at + 3)
            .ok_or_else(|| LspError::Protocol("invalid URI escape".into()))?;
        if !encoded.iter().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(LspError::Protocol("invalid URI escape".into()));
        }
        remaining = &remaining[at + 3..];
    }
    // Encoded traversal and separators must be normalized by the materialization
    // layer; accepting them here could defeat lexical workspace isolation.
    let lower = uri.to_ascii_lowercase();
    if lower.contains("%2f")
        || lower.contains("%5c")
        || lower.contains("%2e")
        || lower.contains("%00")
    {
        return Err(LspError::Protocol("ambiguous encoded URI path".into()));
    }
    Ok(())
}
fn uri_is_within(uri: &str, root: &str) -> bool {
    uri == root
        || uri
            .strip_prefix(root.trim_end_matches('/'))
            .is_some_and(|suffix| suffix.starts_with('/'))
}

/// One shared immutable session per scope/snapshot/analyzer configuration. Pool
/// handles multiplex requests; a changed snapshot creates an isolated session.
#[derive(Default)]
pub struct SharedLanguageServices {
    sessions: Mutex<BTreeMap<SessionKey, Arc<Mutex<LspSession>>>>,
}
impl SharedLanguageServices {
    pub fn get_or_connect(
        &self,
        config: SessionConfig,
        connect: impl FnOnce() -> Result<Box<dyn MessageTransport>, LspError>,
    ) -> Result<Arc<Mutex<LspSession>>, LspError> {
        config.validate()?;
        let mut sessions = self.sessions.lock().map_err(|_| LspError::Poisoned)?;
        if let Some(session) = sessions.get(&config.key) {
            return Ok(Arc::clone(session));
        }
        if sessions.len() >= 64 {
            return Err(LspError::Capacity);
        }
        let key = config.key.clone();
        let session = Arc::new(Mutex::new(LspSession::new(config, connect()?)));
        sessions.insert(key, Arc::clone(&session));
        Ok(session)
    }
    pub fn remove(&self, key: &SessionKey) -> Result<bool, LspError> {
        Ok(self
            .sessions
            .lock()
            .map_err(|_| LspError::Poisoned)?
            .remove(key)
            .is_some())
    }
    pub fn len(&self) -> Result<usize, LspError> {
        Ok(self.sessions.lock().map_err(|_| LspError::Poisoned)?.len())
    }
    pub fn is_empty(&self) -> Result<bool, LspError> {
        Ok(self.len()? == 0)
    }
}

/// Explicit supplement outcome. Unmapped/external targets remain uncertainty;
/// applying queried facts never clears broad semantic coverage requirements.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SupplementReport {
    pub added_edges: usize,
    pub mapped_locations: usize,
    pub unmapped_locations: usize,
}
impl LspSession {
    /// Add version-bound semantic edges to an independently built structural
    /// index. Both source content and result attribution must match. Publication
    /// is atomic: failures leave `index` unchanged.
    pub fn supplement_index(
        &self,
        index: &mut crate::index::SourceIndex,
        result: &SemanticResult,
    ) -> Result<SupplementReport, LspError> {
        use crate::index::{EdgeKind, EdgeProvenance, IndexEdge, OccurrenceKind, Resolution};
        index
            .validate_generation()
            .map_err(|_| LspError::StaleResult)?;
        if !Arc::ptr_eq(&result.origin, &self.marker) {
            return Err(LspError::StaleResult);
        }
        result.validate_snapshot(
            &index.scope,
            index.snapshot,
            self.config.key.server_generation,
        )?;
        if self.document_binding(&result.binding.document_uri)? != result.binding {
            return Err(LspError::StaleResult);
        }
        let source_path = self.index_path(index, &result.binding.document_uri)?;
        let source_doc = &self.documents[&result.binding.document_uri];
        let source_offset = utf16_to_byte(&source_doc.text, result.position)?;
        let source_occurrence = index
            .occurrences
            .iter()
            .filter(|occurrence| {
                occurrence.path == source_path
                    && occurrence.span.start_byte <= source_offset
                    && source_offset < occurrence.span.end_byte
            })
            .min_by_key(|occurrence| occurrence.span.end_byte - occurrence.span.start_byte);
        let source_symbol = index_node_at(index, &source_path, source_offset)
            .or_else(|| {
                source_occurrence.and_then(|occurrence| index.nodes.get(&occurrence.container))
            })
            .ok_or(LspError::StaleResult)?;
        let source_container = source_occurrence.map_or_else(
            || source_symbol.id.clone(),
            |occurrence| occurrence.container.clone(),
        );
        let source_span =
            source_occurrence.map_or(source_symbol.span, |occurrence| occurrence.span);
        let source_id = if source_occurrence.is_some()
            && matches!(
                result.query,
                SemanticQuery::References | SemanticQuery::Implementation
            ) {
            // A references/implementation response identifies consumers of the
            // queried *referent*, not of its enclosing function. Lexical targets
            // are insufficient: require an earlier exact definition query.
            let targets: BTreeSet<_> = index
                .edges
                .iter()
                .filter(|edge| {
                    edge.provenance.path == source_path
                        && edge.provenance.span == source_span
                        && edge.provenance.resolution == Resolution::Analyzer
                        && edge.provenance.extractor
                            == format!(
                                "lsp/3.17:{}:textDocument/definition",
                                self.config.key.server_generation
                            )
                })
                .map(|edge| edge.to.clone())
                .collect();
            if targets.len() != 1 {
                return Err(LspError::Unsupported(
                    "reference-site supplement requires one prior version-bound definition target"
                        .into(),
                ));
            }
            targets.into_iter().next().expect("one target checked")
        } else {
            if matches!(
                result.query,
                SemanticQuery::References
                    | SemanticQuery::Implementation
                    | SemanticQuery::IncomingCalls
                    | SemanticQuery::OutgoingCalls
            ) && !source_symbol.name_span.is_some_and(|span| {
                span.start_byte <= source_offset && source_offset < span.end_byte
            }) {
                return Err(LspError::Unsupported(
                    "semantic source is not an exact indexed declaration name".into(),
                ));
            }
            source_symbol.id.clone()
        };
        let mut next = index.clone();
        let mut report = SupplementReport::default();
        for location in &result.locations {
            if result.location_bindings.get(&location.uri)
                != self.document_binding(&location.uri).ok().as_ref()
                || !result.location_bindings.contains_key(&location.uri)
            {
                report.unmapped_locations += 1;
                continue;
            }
            let path = match self.index_path(index, &location.uri) {
                Ok(path) => path,
                Err(_) => {
                    report.unmapped_locations += 1;
                    continue;
                }
            };
            let (start, end) = self.resolve_location(location)?;
            let target = index_declaration_at(index, &path, start, end).or_else(|| {
                if result.query != SemanticQuery::References {
                    return None;
                }
                index
                    .occurrences
                    .iter()
                    .find(|occurrence| {
                        occurrence.path == path
                            && occurrence.span.start_byte == start
                            && occurrence.span.end_byte == end
                    })
                    .and_then(|occurrence| index.nodes.get(&occurrence.container))
            });
            let Some(target) = target else {
                report.unmapped_locations += 1;
                continue;
            };
            if end > target.span.end_byte {
                report.unmapped_locations += 1;
                continue;
            }
            let (from, to, kind, edge_path, span) =
                match result.query {
                    SemanticQuery::Definition => {
                        let kind = source_occurrence.map_or(EdgeKind::Reference, |occurrence| {
                            match occurrence.kind {
                                OccurrenceKind::Call => EdgeKind::Call,
                                OccurrenceKind::Type => EdgeKind::Type,
                                OccurrenceKind::Implements => EdgeKind::Implements,
                                OccurrenceKind::Import => EdgeKind::Import,
                                _ => EdgeKind::Reference,
                            }
                        });
                        (
                            source_container.clone(),
                            target.id.clone(),
                            kind,
                            source_path.clone(),
                            source_span,
                        )
                    }
                    SemanticQuery::TypeDefinition => (
                        source_container.clone(),
                        target.id.clone(),
                        EdgeKind::Type,
                        source_path.clone(),
                        source_span,
                    ),
                    SemanticQuery::Implementation => (
                        target.id.clone(),
                        source_id.clone(),
                        EdgeKind::Implements,
                        path.clone(),
                        target.span,
                    ),
                    SemanticQuery::References => {
                        let occurrence = index
                            .occurrences
                            .iter()
                            .filter(|occurrence| {
                                occurrence.path == path
                                    && occurrence.span.start_byte <= start
                                    && start < occurrence.span.end_byte
                            })
                            .min_by_key(|occurrence| {
                                occurrence.span.end_byte - occurrence.span.start_byte
                            });
                        let Some(occurrence) = occurrence else {
                            if target.id == source_id {
                                report.mapped_locations += 1;
                                continue;
                            }
                            report.unmapped_locations += 1;
                            continue;
                        };
                        let kind = if occurrence.kind == OccurrenceKind::Call {
                            EdgeKind::Call
                        } else {
                            EdgeKind::Reference
                        };
                        (
                            occurrence.container.clone(),
                            source_id.clone(),
                            kind,
                            path.clone(),
                            occurrence.span,
                        )
                    }
                    SemanticQuery::IncomingCalls => (
                        target.id.clone(),
                        source_id.clone(),
                        EdgeKind::Call,
                        path.clone(),
                        target.span,
                    ),
                    SemanticQuery::OutgoingCalls => (
                        source_id.clone(),
                        target.id.clone(),
                        EdgeKind::Call,
                        source_path.clone(),
                        source_span,
                    ),
                    SemanticQuery::PrepareCallHierarchy => {
                        report.mapped_locations += 1;
                        continue;
                    }
                };
            if from != to {
                let inserted = next.edges.insert(IndexEdge {
                    from,
                    to,
                    kind,
                    provenance: EdgeProvenance {
                        snapshot: index.snapshot,
                        extractor: format!(
                            "lsp/3.17:{}:{}",
                            self.config.key.server_generation,
                            result.query.method()
                        ),
                        path: edge_path,
                        span,
                        resolution: Resolution::Analyzer,
                    },
                });
                report.added_edges += usize::from(inserted);
            }
            report.mapped_locations += 1;
        }
        // A response can supplement known relationships without proving coverage.
        // Unmapped dependencies are explicit in both the report and generation.
        next.issues.insert(crate::index::IndexIssue{path:source_path.clone(),span:source_span,kind:crate::index::IssueKind::SemanticAnalysisUnavailable,detail:format!("query-only LSP facts: generation={}, operation={}, mapped={}, unmapped={}; no exhaustive semantic coverage claim",self.config.key.server_generation,result.query.method(),report.mapped_locations,report.unmapped_locations)});
        if report.unmapped_locations > 0 {
            next.issues.insert(crate::index::IndexIssue{path:source_path,span:source_span,kind:crate::index::IssueKind::SemanticAnalysisUnavailable,detail:format!("{} LSP target locations are outside indexed immutable documents or lack a structural declaration",report.unmapped_locations)});
        }
        next.refresh_generation();
        next.validate_generation()
            .map_err(|_| LspError::StaleResult)?;
        *index = next;
        Ok(report)
    }
    fn index_path(&self, index: &crate::index::SourceIndex, uri: &str) -> Result<String, LspError> {
        let doc = self
            .documents
            .get(uri)
            .filter(|doc| doc.open)
            .ok_or_else(|| LspError::UnknownDocument(uri.into()))?;
        let suffix = uri
            .strip_prefix(self.config.root_uri.trim_end_matches('/'))
            .and_then(|suffix| suffix.strip_prefix('/'))
            .ok_or(LspError::StaleResult)?;
        let mut path = Vec::new();
        let bytes = suffix.as_bytes();
        let mut at = 0;
        while at < bytes.len() {
            if bytes[at] == b'%' {
                let hex = bytes
                    .get(at + 1..at + 3)
                    .ok_or_else(|| LspError::Protocol("invalid percent encoding".into()))?;
                let hex = std::str::from_utf8(hex)
                    .map_err(|_| LspError::Protocol("invalid percent encoding".into()))?;
                path.push(
                    u8::from_str_radix(hex, 16)
                        .map_err(|_| LspError::Protocol("invalid percent encoding".into()))?,
                );
                at += 3;
            } else {
                path.push(bytes[at]);
                at += 1;
            }
        }
        let path = String::from_utf8(path)
            .map_err(|_| LspError::Protocol("invalid URI path encoding".into()))?;
        let file = index.files.get(&path).ok_or(LspError::StaleResult)?;
        use crate::index::Language;
        let matching_language = match file.language {
            Some(Language::TypeScript) => doc.language == "typescript",
            Some(Language::Tsx) => doc.language == "typescriptreact",
            Some(Language::JavaScript) => {
                matches!(doc.language.as_str(), "javascript" | "javascriptreact")
            }
            Some(Language::Python) => doc.language == "python",
            Some(Language::Go) => doc.language == "go",
            Some(Language::Rust) => doc.language == "rust",
            None => false,
        };
        if !matching_language {
            return Err(LspError::Unsupported(
                "index and language-server document adapters differ".into(),
            ));
        }
        if file.content_digest != Digest::of("celetixo/source-bytes/v1", doc.text.as_bytes()) {
            return Err(LspError::StaleResult);
        }
        if index
            .nodes
            .values()
            .filter(|node| node.path == path)
            .any(|node| {
                node.name_span.is_some_and(|span| {
                    doc.text.as_bytes().get(span.start_byte..span.end_byte)
                        != Some(node.name.as_bytes())
                })
            })
        {
            return Err(LspError::StaleResult);
        }
        Ok(path)
    }
}
fn index_node_at<'a>(
    index: &'a crate::index::SourceIndex,
    path: &str,
    offset: usize,
) -> Option<&'a crate::index::IndexNode> {
    use crate::index::NodeKind;
    index
        .nodes
        .values()
        .filter(|node| {
            node.path == path
                && node.span.start_byte <= offset
                && offset < node.span.end_byte
                && !matches!(
                    node.kind,
                    NodeKind::File | NodeKind::Module | NodeKind::Scope
                )
        })
        .min_by_key(|node| node.span.end_byte - node.span.start_byte)
}
fn index_declaration_at<'a>(
    index: &'a crate::index::SourceIndex,
    path: &str,
    start: usize,
    end: usize,
) -> Option<&'a crate::index::IndexNode> {
    let mut candidates = index.nodes.values().filter(|node| {
        node.path == path
            && node
                .name_span
                .is_some_and(|span| span.start_byte == start && span.end_byte == end)
    });
    let first = candidates.next()?;
    if candidates.next().is_some() {
        None
    } else {
        Some(first)
    }
}

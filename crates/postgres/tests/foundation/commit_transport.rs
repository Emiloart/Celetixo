//! Test-only authenticated Unix relay. It discards a real server COMMIT reply;
//! it never logs credentials, protocol payloads or manufactures a SQL response.
use super::*;
use std::io::{Read, Write};
use std::net::Shutdown;
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::atomic::{AtomicBool, Ordering};

struct LostReply {
    directory: tempfile::TempDir,
    armed: Arc<AtomicBool>,
    observed: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl LostReply {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let listener = UnixListener::bind(directory.path().join(".s.PGSQL.5432")).unwrap();
        listener.set_nonblocking(true).unwrap();
        let armed = Arc::new(AtomicBool::new(false));
        let observed = Arc::new(AtomicBool::new(false));
        let stop = Arc::new(AtomicBool::new(false));
        let (arm, saw, stopping) = (armed.clone(), observed.clone(), stop.clone());
        let upstream = fixture().host.join(".s.PGSQL.5432");
        let thread = std::thread::spawn(move || {
            let mut connections = vec![];
            while !stopping.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((client, _)) => {
                        let server = UnixStream::connect(&upstream).unwrap();
                        let arm = arm.clone();
                        let saw = saw.clone();
                        connections.push(std::thread::spawn(move || {
                            relay(client, server, &arm, &saw)
                        }));
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(2))
                    }
                    Err(error) => panic!("test relay accept failed: {}", error.kind()),
                }
            }
            for connection in connections {
                connection.join().unwrap();
            }
        });
        Self {
            directory,
            armed,
            observed,
            stop,
            thread: Some(thread),
        }
    }
    fn connection(&self, role: &str) -> UnixConnection {
        let configuration = fixture();
        let role = &configuration.roles[role];
        let password = std::fs::read_to_string(&role.password_file).unwrap();
        UnixConnection::new(
            self.directory.path(),
            &configuration.database,
            &role.user,
            password.trim().as_bytes(),
        )
        .unwrap()
    }
}
impl Drop for LostReply {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        self.thread.take().unwrap().join().unwrap();
    }
}
fn relay(
    mut client: UnixStream,
    mut server: UnixStream,
    armed: &AtomicBool,
    observed: &AtomicBool,
) {
    let mut reader = client.try_clone().unwrap();
    let mut writer = server.try_clone().unwrap();
    let uplink = std::thread::spawn(move || std::io::copy(&mut reader, &mut writer));
    // NoTls startup produces ordinary typed server messages, including SCRAM.
    // Bound allocations even in this trusted local test transport.
    let downlink = || -> std::io::Result<()> {
        loop {
            let mut header = [0_u8; 5];
            server.read_exact(&mut header)?;
            let length =
                usize::try_from(u32::from_be_bytes(header[1..].try_into().unwrap())).unwrap();
            if !(4..=16 * 1024 * 1024).contains(&length) {
                return Err(std::io::ErrorKind::InvalidData.into());
            }
            let mut body = vec![0; length - 4];
            server.read_exact(&mut body)?;
            if header[0] == b'C' && body == b"COMMIT\0" && armed.swap(false, Ordering::SeqCst) {
                observed.store(true, Ordering::SeqCst);
                return Ok(());
            }
            client.write_all(&header)?;
            client.write_all(&body)?;
        }
    }();
    // EOF is expected when an ordinary fresh authority connection is dropped,
    // and when the intentionally lost reply makes the client close its socket.
    assert!(
        downlink.is_ok()
            || downlink.is_err_and(|error| matches!(
                error.kind(),
                std::io::ErrorKind::UnexpectedEof
                    | std::io::ErrorKind::BrokenPipe
                    | std::io::ErrorKind::ConnectionReset
            ))
    );
    let _ = client.shutdown(Shutdown::Both);
    let _ = server.shutdown(Shutdown::Both);
    let _ = uplink.join().unwrap();
}

#[test]
#[ignore = "requires the reviewed authenticated PostgreSQL fixture"]
fn actual_server_commit_reply_loss_reconciles_the_original_durable_receipt() {
    let mut h = Harness::new("native-transport-loss");
    h.ready_at("a", &h.base, "x", 10, h.intent("x"));
    let request = h.request("a", "transport-loss");
    let relay = LostReply::new();
    let backend = Arc::new(
        PostgresAuthority::connect(
            relay.connection("runtime"),
            relay.connection("runtime_other"),
        )
        .unwrap(),
    );
    let (engine, admin) = Engine::with_authority_backend(backend);
    let worker = admin
        .authenticate_existing_session(h.scope(), 1, |_, _| Ok(()))
        .unwrap();
    relay.armed.store(true, Ordering::SeqCst);
    assert_eq!(
        engine.commit_prepared(&worker, h.scope(), request.clone()),
        Err(authority_error(AuthorityFailure::CommitIndeterminate))
    );
    assert!(
        relay.observed.load(Ordering::SeqCst),
        "real server CommandComplete was received before transport close"
    );
    assert_eq!(h.counts(), vec![1, 1, 1, 1, 1, 0]);
    let committed = h.backend.inspect_native(h.scope()).unwrap();
    let operation = &committed.records.operations[&request.key];
    assert_eq!(operation.request, request);
    assert!(operation.outcome.is_ok());
    assert_eq!(
        committed.records.repository.as_ref().unwrap().history[0].receipt,
        operation.outcome.clone().unwrap()
    );
    h.reopen();
    let receipt = h.commit(request.clone()).unwrap();
    assert_eq!(h.commit(request), Ok(receipt));
    assert_eq!(h.counts(), vec![1, 1, 1, 1, 1, 0]);
    let replayed = h.backend.inspect_native(h.scope()).unwrap();
    assert_eq!(replayed.records.repository, committed.records.repository);
    assert_eq!(
        replayed.records.transactions,
        committed.records.transactions
    );
    assert_eq!(replayed.records.operations, committed.records.operations);
    drop(worker);
    drop(admin);
    drop(engine);
    drop(relay);
}

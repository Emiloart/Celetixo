//! Trusted, pinned OCI entrypoint. This program grants no Engine authority.
//! The parent verifies effective isolation and admission before releasing stdin.
use std::io::{Read, Write};
use std::process::Command;

#[cfg(unix)]
fn main() {
    use std::os::unix::process::CommandExt;
    let mut arguments = std::env::args_os().skip(1);
    let first = arguments.next();
    if first.as_deref() == Some(std::ffi::OsStr::new("--probe")) {
        let mode = arguments
            .next()
            .and_then(|s| s.into_string().ok())
            .unwrap_or_default();
        probe(&mode, arguments.collect());
        return;
    }
    if first.as_deref() != Some(std::ffi::OsStr::new("--")) {
        std::process::exit(125);
    }
    let Some(executable) = arguments.next() else {
        std::process::exit(125);
    };
    if !std::path::Path::new(&executable).is_absolute() {
        std::process::exit(125);
    }
    let mut stdout = std::io::stdout().lock();
    if stdout.write_all(b"CELETIXO_EXEC_READY_V1\n").is_err() || stdout.flush().is_err() {
        std::process::exit(125);
    }
    drop(stdout);
    let mut release = [0_u8; 1];
    if std::io::stdin().read_exact(&mut release).is_err() || release != [1] {
        std::process::exit(125);
    }
    // stdin becomes EOF after the parent releases this single check. The exact
    // OCI environment and working directory are inherited; no shell is involved.
    let error = Command::new(executable).args(arguments).exec();
    let _ = writeln!(std::io::stderr(), "trusted execution entrypoint: {error}");
    std::process::exit(125);
}

/// Trusted, fixed bounded adversarial fixtures. These are never repository
/// scripts and grant no additional syscall, mount, resource or Engine rights.
#[cfg(unix)]
fn probe(mode: &str, arguments: Vec<std::ffi::OsString>) {
    use std::fs::{self, File, OpenOptions};
    use std::io::Write;
    use std::time::{Duration, Instant};
    let assert_denied = |result: std::io::Result<File>| {
        assert!(result.is_err(), "forbidden filesystem access succeeded");
    };
    match mode {
        "boundaries" => {
            assert_denied(OpenOptions::new().write(true).open("/src/main.go"));
            assert_denied(
                OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open("/outside-scratch"),
            );
            for path in arguments {
                assert_denied(File::open(path));
            }
            assert!(!std::path::Path::new("/var/run/docker.sock").exists());
            assert!(!std::path::Path::new("/run/user/1000/bus").exists());
            assert_eq!(
                std::env::vars().collect::<Vec<_>>(),
                vec![("HOME".into(), "/scratch".into())]
            );
            assert!(
                std::net::TcpStream::connect_timeout(
                    &"1.1.1.1:443".parse().unwrap(),
                    Duration::from_millis(50)
                )
                .is_err()
            );
            fs::write("/scratch/probe", b"declared scratch works").unwrap();
            assert_eq!(
                fs::read("/scratch/probe").unwrap(),
                b"declared scratch works"
            );
            println!("filesystem-environment-network-boundaries-ok");
        }
        "disk" => {
            let mut denied = false;
            for n in 0..64 {
                match fs::write(format!("/scratch/chunk-{n}"), [0_u8; 4096]) {
                    Ok(()) => {}
                    Err(_) => {
                        denied = true;
                        break;
                    }
                }
            }
            assert!(denied, "scratch disk limit absent");
            println!("scratch-disk-limit-ok");
        }
        "fds" => {
            let mut files = Vec::new();
            let mut denied = false;
            for _ in 0..128 {
                match File::open("/dev/null") {
                    Ok(file) => files.push(file),
                    Err(_) => {
                        denied = true;
                        break;
                    }
                }
            }
            assert!(denied, "file-descriptor limit absent");
            println!("file-descriptor-limit-ok");
        }
        "pids" => {
            let mut children = Vec::new();
            let mut denied = false;
            for _ in 0..64 {
                match Command::new("/celetixo/exec-init")
                    .args(["--probe", "wait"])
                    .spawn()
                {
                    Ok(child) => children.push(child),
                    Err(_) => {
                        denied = true;
                        break;
                    }
                }
            }
            for child in &mut children {
                let _ = child.kill();
                let _ = child.wait();
            }
            assert!(denied, "process limit absent");
            println!("process-limit-ok");
        }
        "memory" => {
            // Touched pages force accounting; the configured 64MiB cgroup must
            // stop this process before the finite 128MiB fixture can complete.
            let mut chunks = Vec::new();
            for _ in 0..128 {
                chunks.push(vec![0xa5_u8; 1024 * 1024]);
                std::hint::black_box(chunks.last().unwrap());
            }
            panic!("memory limit absent");
        }
        "cpu" => {
            let start = Instant::now();
            while start.elapsed() < Duration::from_secs(12) {
                std::hint::spin_loop();
            }
            panic!("CPU/wall limit absent");
        }
        "children" => {
            let mut child = Command::new("/celetixo/exec-init")
                .args(["--probe", "wait"])
                .spawn()
                .unwrap();
            println!("child-pid={}", child.id());
            std::io::stdout().flush().unwrap();
            child.wait().unwrap();
        }
        "wait" => std::thread::sleep(Duration::from_secs(12)),
        "output" => {
            for _ in 0..256 {
                println!("{}", "x".repeat(1024));
            }
        }
        _ => std::process::exit(125),
    }
}

#[cfg(not(unix))]
fn main() {
    std::process::exit(125);
}

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use kulkan_ipc::{Message, read_message, write_message};
use kulkan_runtime::ServerProcess;

static NEXT_TEST_DIR: AtomicUsize = AtomicUsize::new(0);

#[test]
fn client_starts_the_child_server_and_stops_it_cleanly() {
    let runtime = temporary_runtime_directory();
    let socket_path = runtime.join("server.sock");
    let executable = PathBuf::from(env!("CARGO_BIN_EXE_kulkan-server"));

    let mut server = ServerProcess::start(&executable, &socket_path, Duration::from_secs(2))
        .expect("child server should become ready");
    assert!(socket_path.exists());
    server.ping().expect("server should answer Ping");

    server.shutdown().expect("server should shut down cleanly");
    assert!(!server.is_running().unwrap());
    assert!(!socket_path.exists());

    let _ = std::fs::remove_dir_all(runtime);
}

#[test]
fn a_failed_server_start_can_be_retried_with_the_same_socket_path() {
    let runtime = temporary_runtime_directory();
    let socket_path = runtime.join("server.sock");
    let missing_executable = runtime.join("missing-server");
    let server_executable = PathBuf::from(env!("CARGO_BIN_EXE_kulkan-server"));

    assert!(
        ServerProcess::start(
            &missing_executable,
            &socket_path,
            Duration::from_millis(100)
        )
        .is_err()
    );

    let mut server = ServerProcess::start(&server_executable, &socket_path, Duration::from_secs(2))
        .expect("a later retry should be able to start the server");
    server.shutdown().unwrap();
    let _ = std::fs::remove_dir_all(runtime);
}

#[test]
fn startup_deadline_covers_an_unresponsive_server_handshake() {
    let runtime = temporary_runtime_directory();
    let socket_path = runtime.join("server.sock");
    let executable = PathBuf::from(env!("CARGO_BIN_EXE_kulkan-stalled-server"));
    let (result_sender, result_receiver) = mpsc::channel();
    let startup_started = Instant::now();

    let startup_thread = thread::spawn(move || {
        let result = ServerProcess::start(&executable, &socket_path, Duration::from_millis(150));
        let _ = result_sender.send(result);
    });
    let early_result = result_receiver.recv_timeout(Duration::from_millis(400));
    let returned_before_deadline = early_result.is_ok();
    let result = match early_result {
        Ok(result) => result,
        Err(_) => result_receiver
            .recv_timeout(Duration::from_secs(2))
            .expect("stalled fixture should eventually exit"),
    };
    startup_thread.join().unwrap();

    assert!(
        returned_before_deadline,
        "startup took {:?} despite its 150ms timeout",
        startup_started.elapsed()
    );
    assert!(result.is_err());
    let _ = std::fs::remove_file(runtime.join("server.sock"));
    let _ = std::fs::remove_dir_all(runtime);
}

#[test]
fn startup_does_not_adopt_a_live_server_from_an_earlier_client() {
    let runtime = temporary_runtime_directory();
    let socket_path = runtime.join("server.sock");
    let server_executable = PathBuf::from(env!("CARGO_BIN_EXE_kulkan-server"));
    let mut orphan = Command::new(&server_executable)
        .arg("--socket")
        .arg(&socket_path)
        .spawn()
        .unwrap();
    wait_for_ping(&socket_path);

    let mut managed =
        ServerProcess::start(&server_executable, &socket_path, Duration::from_secs(2))
            .expect("client should replace an orphaned server with its own child");
    thread::sleep(Duration::from_millis(50));
    let owns_live_child = managed.is_running().unwrap() && managed.ping().is_ok();
    if owns_live_child {
        managed.shutdown().unwrap();
    } else {
        send_shutdown(&socket_path);
    }
    orphan.wait().unwrap();

    assert!(
        owns_live_child,
        "client adopted an already-running old server"
    );
    let _ = std::fs::remove_dir_all(runtime);
}

fn wait_for_ping(socket_path: &std::path::Path) {
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        if let Ok(mut stream) = std::os::unix::net::UnixStream::connect(socket_path) {
            if write_message(&mut stream, &Message::Ping).is_ok()
                && read_message(&mut stream).is_ok_and(|message| message == Message::Pong)
            {
                return;
            }
        }
        thread::sleep(Duration::from_millis(10));
    }
    panic!("server did not become ready");
}

fn send_shutdown(socket_path: &std::path::Path) {
    if let Ok(mut stream) = std::os::unix::net::UnixStream::connect(socket_path) {
        let _ = write_message(&mut stream, &Message::Shutdown);
        let _ = read_message(&mut stream);
    }
}

fn temporary_runtime_directory() -> PathBuf {
    let suffix = NEXT_TEST_DIR.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "kulkan-runtime-test-{}-{suffix}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir(&path).unwrap();
    path
}

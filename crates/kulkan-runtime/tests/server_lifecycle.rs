use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

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

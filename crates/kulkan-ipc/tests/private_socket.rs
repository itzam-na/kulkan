use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use kulkan_ipc::{bind_private_socket, validate_peer};

static NEXT_TEST_DIR: AtomicUsize = AtomicUsize::new(0);

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new() -> Self {
        let suffix = NEXT_TEST_DIR.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("kulkan-ipc-test-{}-{suffix}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn binds_a_private_socket_and_accepts_a_same_user_peer() {
    let directory = TestDirectory::new();
    let socket_directory = directory.0.join("runtime");
    let socket_path = socket_directory.join("server.sock");

    let listener = bind_private_socket(&socket_path).unwrap();
    assert_eq!(
        fs::metadata(&socket_directory)
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    assert_eq!(
        fs::metadata(&socket_path).unwrap().permissions().mode() & 0o777,
        0o600
    );

    let client = UnixStream::connect(&socket_path).unwrap();
    let (server, _) = listener.accept().unwrap();
    validate_peer(&client).unwrap();
    validate_peer(&server).unwrap();
}

#[test]
fn tightens_permissions_on_an_existing_user_owned_socket_directory() {
    let directory = TestDirectory::new();
    let socket_directory = directory.0.join("runtime");
    fs::create_dir(&socket_directory).unwrap();
    fs::set_permissions(&socket_directory, fs::Permissions::from_mode(0o755)).unwrap();
    let socket_path = socket_directory.join("server.sock");

    let _listener = bind_private_socket(&socket_path).unwrap();

    assert_eq!(
        fs::metadata(&socket_directory)
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
}

#[test]
fn removes_only_a_stale_socket_after_connecting_to_check_for_a_live_owner() {
    let directory = TestDirectory::new();
    let socket_path = directory.0.join("runtime").join("server.sock");
    let stale = bind_private_socket(&socket_path).unwrap();
    drop(stale);

    let recovered = bind_private_socket(&socket_path).unwrap();
    assert!(socket_path.exists());
    drop(recovered);

    let live = bind_private_socket(&socket_path).unwrap();
    let second_bind = bind_private_socket(&socket_path).unwrap_err();
    assert_eq!(second_bind.kind(), std::io::ErrorKind::AddrInUse);
    assert!(socket_path.exists());
    drop(live);
}

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::RecvTimeoutError;
use std::time::Duration;

use kulkan_runtime::{ClientInstance, activate_existing_client};

static NEXT_TEST_DIR: AtomicUsize = AtomicUsize::new(0);

#[test]
fn a_second_launch_activates_the_existing_client_instead_of_claiming_another_instance() {
    let runtime = temporary_runtime_directory();
    let socket_path = runtime.join("client.sock");

    assert!(!activate_existing_client(&socket_path).unwrap());
    let (instance, activations) = ClientInstance::start(&socket_path).unwrap();

    assert!(activate_existing_client(&socket_path).unwrap());
    assert_eq!(activations.recv_timeout(Duration::from_secs(1)), Ok(()));
    assert!(ClientInstance::start(&socket_path).is_err());

    drop(instance);
    assert!(!socket_path.exists());
    let _ = std::fs::remove_dir_all(runtime);
}

#[test]
fn an_unanswered_launch_does_not_claim_a_running_client() {
    let runtime = temporary_runtime_directory();
    let socket_path = runtime.join("client.sock");
    let (instance, activations) = ClientInstance::start(&socket_path).unwrap();

    drop(instance);

    assert_eq!(
        activations.recv_timeout(Duration::from_millis(10)),
        Err(RecvTimeoutError::Disconnected)
    );
    let _ = std::fs::remove_dir_all(runtime);
}

fn temporary_runtime_directory() -> PathBuf {
    let suffix = NEXT_TEST_DIR.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "kulkan-client-instance-test-{}-{suffix}",
        std::process::id(),
    ));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir(&path).unwrap();
    path
}

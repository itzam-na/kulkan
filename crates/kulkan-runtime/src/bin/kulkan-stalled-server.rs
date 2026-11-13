use std::io::Read;
use std::os::unix::net::UnixListener;
use std::path::PathBuf;
use std::thread;
use std::time::Duration;

fn main() {
    let mut arguments = std::env::args_os().skip(1);
    assert_eq!(
        arguments.next().as_deref(),
        Some(std::ffi::OsStr::new("--socket"))
    );
    let socket_path = PathBuf::from(arguments.next().expect("socket path"));
    let listener = UnixListener::bind(socket_path).unwrap();
    let (mut peer, _) = listener.accept().unwrap();
    let mut prefix = [0; 4];
    let _ = peer.read_exact(&mut prefix);
    thread::sleep(Duration::from_secs(1));
}

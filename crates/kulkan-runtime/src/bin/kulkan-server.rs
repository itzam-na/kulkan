use std::path::PathBuf;

fn main() {
    let mut arguments = std::env::args_os().skip(1);
    let Some(option) = arguments.next() else {
        eprintln!("usage: kulkan-server --socket PATH");
        std::process::exit(2);
    };
    let Some(path) = arguments.next() else {
        eprintln!("usage: kulkan-server --socket PATH");
        std::process::exit(2);
    };
    if option != "--socket" || arguments.next().is_some() {
        eprintln!("usage: kulkan-server --socket PATH");
        std::process::exit(2);
    }

    let socket_path = PathBuf::from(path);
    if let Err(error) = kulkan_runtime::serve(&socket_path) {
        eprintln!("Kulkán server failed: {error}");
        std::process::exit(1);
    }
}

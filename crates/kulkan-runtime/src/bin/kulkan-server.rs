use std::path::PathBuf;

fn main() {
    let mut arguments = std::env::args_os().skip(1);
    let mut socket_path = None;
    let mut audio_sink = None;
    let mut library_state_path = None;
    let mut start_paused = false;
    while let Some(option) = arguments.next() {
        if option == "--start-paused" && !start_paused {
            start_paused = true;
            continue;
        }
        let Some(value) = arguments.next() else {
            eprintln!(
                "usage: kulkan-server --socket PATH [--audio-sink NAME] [--library-state PATH] [--start-paused]"
            );
            std::process::exit(2);
        };
        match option.to_str() {
            Some("--socket") if socket_path.is_none() => socket_path = Some(PathBuf::from(value)),
            Some("--audio-sink") if audio_sink.is_none() => {
                audio_sink = value.into_string().ok();
                if audio_sink.is_none() {
                    eprintln!("the audio sink name must be valid UTF-8");
                    std::process::exit(2);
                }
            }
            Some("--library-state") if library_state_path.is_none() => {
                library_state_path = Some(PathBuf::from(value));
            }
            _ => {
                eprintln!(
                    "usage: kulkan-server --socket PATH [--audio-sink NAME] [--library-state PATH] [--start-paused]"
                );
                std::process::exit(2);
            }
        }
    }

    let Some(socket_path) = socket_path else {
        eprintln!(
            "usage: kulkan-server --socket PATH [--audio-sink NAME] [--library-state PATH] [--start-paused]"
        );
        std::process::exit(2);
    };
    serve(
        &socket_path,
        audio_sink.as_deref(),
        library_state_path.as_deref(),
        start_paused,
    );
}

fn serve(
    socket_path: &std::path::Path,
    audio_sink: Option<&str>,
    state_path: Option<&std::path::Path>,
    start_paused: bool,
) {
    let result = if start_paused {
        kulkan_runtime::serve_with_library_state_paused(socket_path, audio_sink, state_path)
    } else {
        kulkan_runtime::serve_with_library_state(socket_path, audio_sink, state_path)
    };
    if let Err(error) = result {
        eprintln!("Kulkán server failed: {error}");
        std::process::exit(1);
    }
}

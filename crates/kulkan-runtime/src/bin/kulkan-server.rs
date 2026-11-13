use std::path::PathBuf;

fn main() {
    let mut arguments = std::env::args_os().skip(1);
    let mut socket_path = None;
    let mut audio_sink = None;
    while let Some(option) = arguments.next() {
        let Some(value) = arguments.next() else {
            eprintln!("usage: kulkan-server --socket PATH [--audio-sink NAME]");
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
            _ => {
                eprintln!("usage: kulkan-server --socket PATH [--audio-sink NAME]");
                std::process::exit(2);
            }
        }
    }

    let Some(socket_path) = socket_path else {
        eprintln!("usage: kulkan-server --socket PATH [--audio-sink NAME]");
        std::process::exit(2);
    };
    if let Err(error) = kulkan_runtime::serve_with_audio_sink(&socket_path, audio_sink.as_deref()) {
        eprintln!("Kulkán server failed: {error}");
        std::process::exit(1);
    }
}

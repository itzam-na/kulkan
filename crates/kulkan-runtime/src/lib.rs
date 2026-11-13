use std::io;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::thread;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use kulkan_ipc::{Message, bind_private_socket, read_message, validate_peer, write_message};

/// Runs the local server protocol until the client asks it to shut down.
pub fn serve(socket_path: &Path) -> io::Result<()> {
    let listener = bind_private_socket(socket_path)?;
    let _cleanup = SocketPathCleanup(socket_path.to_path_buf());

    for incoming in listener.incoming() {
        let mut stream = incoming?;
        if validate_peer(&stream).is_err() {
            continue;
        }

        let request = match read_message(&mut stream) {
            Ok(request) => request,
            Err(_) => continue,
        };
        match request {
            Message::Ping => write_message(&mut stream, &Message::Pong)
                .map_err(|error| io::Error::other(error.to_string()))?,
            Message::Shutdown => {
                write_message(&mut stream, &Message::ShutdownAck)
                    .map_err(|error| io::Error::other(error.to_string()))?;
                return Ok(());
            }
            _ => {}
        }
    }
    Ok(())
}

/// Owns the child server process and its private socket connection lifecycle.
pub struct ServerProcess {
    child: Child,
    socket_path: PathBuf,
}

/// Owns the private activation socket for the one running desktop client.
pub struct ClientInstance {
    socket_path: PathBuf,
    shutting_down: Arc<AtomicBool>,
    listener_thread: Option<JoinHandle<()>>,
}

impl ClientInstance {
    /// Claims the client activation socket and returns notifications for new launches.
    pub fn start(socket_path: &Path) -> io::Result<(Self, mpsc::Receiver<()>)> {
        let listener = bind_private_socket(socket_path)?;
        listener.set_nonblocking(true)?;
        let shutting_down = Arc::new(AtomicBool::new(false));
        let thread_shutdown = Arc::clone(&shutting_down);
        let (activation_sender, activation_receiver) = mpsc::channel();
        let listener_thread = thread::spawn(move || {
            while !thread_shutdown.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        if validate_peer(&stream).is_err() {
                            continue;
                        }
                        if stream
                            .set_read_timeout(Some(Duration::from_millis(500)))
                            .is_err()
                        {
                            continue;
                        }
                        if read_message(&mut stream)
                            .is_ok_and(|message| message == Message::Activate)
                            && write_message(&mut stream, &Message::ActivateAck).is_ok()
                        {
                            let _ = activation_sender.send(());
                        }
                    }
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(_) => break,
                }
            }
        });

        Ok((
            Self {
                socket_path: socket_path.to_path_buf(),
                shutting_down,
                listener_thread: Some(listener_thread),
            },
            activation_receiver,
        ))
    }
}

impl Drop for ClientInstance {
    fn drop(&mut self) {
        self.shutting_down.store(true, Ordering::Relaxed);
        if let Some(listener_thread) = self.listener_thread.take() {
            let _ = listener_thread.join();
        }
        let _ = std::fs::remove_file(&self.socket_path);
    }
}

/// Asks an already-running client to present its window.
///
/// Returns `Ok(false)` if no live client currently owns the socket.
pub fn activate_existing_client(socket_path: &Path) -> io::Result<bool> {
    let mut client = match UnixStream::connect(socket_path) {
        Ok(client) => client,
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
            ) =>
        {
            return Ok(false);
        }
        Err(error) => return Err(error),
    };
    validate_peer(&client)?;
    write_message(&mut client, &Message::Activate)
        .map_err(|error| io::Error::other(error.to_string()))?;
    match read_message(&mut client).map_err(|error| io::Error::other(error.to_string()))? {
        Message::ActivateAck => Ok(true),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "existing client returned an unexpected activation response",
        )),
    }
}

impl ServerProcess {
    /// Starts the server executable and waits until its versioned socket responds to Ping.
    pub fn start(
        executable: &Path,
        socket_path: &Path,
        startup_timeout: Duration,
    ) -> io::Result<Self> {
        let mut child = Command::new(executable)
            .arg("--socket")
            .arg(socket_path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
        let deadline = Instant::now() + startup_timeout;

        loop {
            if let Ok(mut server) = connect(socket_path) {
                if write_message(&mut server, &Message::Ping).is_ok()
                    && read_message(&mut server).is_ok_and(|message| message == Message::Pong)
                {
                    return Ok(Self {
                        child,
                        socket_path: socket_path.to_path_buf(),
                    });
                }
            }

            if let Some(status) = child.try_wait()? {
                return Err(io::Error::other(format!(
                    "server exited during startup with {status}"
                )));
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "server did not become ready before the startup timeout",
                ));
            }
            thread::sleep(Duration::from_millis(10));
        }
    }

    /// Confirms the child still serves the current protocol.
    pub fn ping(&self) -> io::Result<()> {
        let mut server = connect(&self.socket_path)?;
        write_message(&mut server, &Message::Ping)
            .map_err(|error| io::Error::other(error.to_string()))?;
        match read_message(&mut server).map_err(|error| io::Error::other(error.to_string()))? {
            Message::Pong => Ok(()),
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "server returned an unexpected response to Ping",
            )),
        }
    }

    /// Requests a clean shutdown and waits for the child to exit.
    pub fn shutdown(&mut self) -> io::Result<()> {
        if self.child.try_wait()?.is_some() {
            return Ok(());
        }

        let mut server = connect(&self.socket_path)?;
        write_message(&mut server, &Message::Shutdown)
            .map_err(|error| io::Error::other(error.to_string()))?;
        match read_message(&mut server).map_err(|error| io::Error::other(error.to_string()))? {
            Message::ShutdownAck => {}
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "server returned an unexpected response to Shutdown",
                ));
            }
        }
        self.wait_until_exit(Duration::from_secs(2))
    }

    /// Reports whether the child process is still running.
    pub fn is_running(&mut self) -> io::Result<bool> {
        Ok(self.child.try_wait()?.is_none())
    }

    fn wait_until_exit(&mut self, timeout: Duration) -> io::Result<()> {
        let deadline = Instant::now() + timeout;
        loop {
            if self.child.try_wait()?.is_some() {
                return Ok(());
            }
            if Instant::now() >= deadline {
                self.child.kill()?;
                let _ = self.child.wait()?;
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "server did not exit after acknowledging Shutdown",
                ));
            }
            thread::sleep(Duration::from_millis(10));
        }
    }
}

impl Drop for ServerProcess {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.shutdown();
        }
    }
}

fn connect(socket_path: &Path) -> io::Result<UnixStream> {
    let stream = UnixStream::connect(socket_path)?;
    validate_peer(&stream)?;
    Ok(stream)
}

struct SocketPathCleanup(PathBuf);

impl Drop for SocketPathCleanup {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

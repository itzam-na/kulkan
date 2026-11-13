use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::thread;
use std::time::Duration;

use gtk::gio;
use gtk::glib;
use gtk::prelude::*;
use kulkan_runtime::{ClientInstance, ServerProcess, activate_existing_client};

const APPLICATION_ID: &str = "org.kulkan.Kulkan";

fn main() -> glib::ExitCode {
    match run() {
        Ok(exit_code) => exit_code,
        Err(error) => {
            eprintln!("Kulkán could not start: {error}");
            glib::ExitCode::FAILURE
        }
    }
}

fn run() -> Result<glib::ExitCode, Box<dyn std::error::Error>> {
    let runtime_directory = runtime_directory();
    let client_socket = runtime_directory.join("client.sock");
    let server_socket = runtime_directory.join("server.sock");

    if activate_existing_client(&client_socket)? {
        return Ok(glib::ExitCode::SUCCESS);
    }

    let (client_instance, activations) = match ClientInstance::start(&client_socket) {
        Ok(instance) => instance,
        Err(first_error) => {
            for _ in 0..10 {
                if activate_existing_client(&client_socket).unwrap_or(false) {
                    return Ok(glib::ExitCode::SUCCESS);
                }
                thread::sleep(Duration::from_millis(20));
            }
            return Err(Box::new(first_error));
        }
    };

    let server_executable = std::env::current_exe()?
        .parent()
        .ok_or("Kulkán executable has no parent directory")?
        .join("kulkan-server");
    let server_state = Rc::new(RefCell::new(ServerState {
        executable: server_executable,
        socket_path: server_socket,
        process: None,
    }));
    let _client_instance = client_instance;
    let application = gtk::Application::builder()
        .application_id(APPLICATION_ID)
        .flags(gio::ApplicationFlags::NON_UNIQUE)
        .build();

    let activation_application = application.clone();
    glib::timeout_add_local(Duration::from_millis(40), move || {
        while activations.try_recv().is_ok() {
            if let Some(window) = activation_application.active_window() {
                window.present();
            } else {
                activation_application.activate();
            }
        }
        glib::ControlFlow::Continue
    });

    let activation_state = Rc::clone(&server_state);
    application.connect_activate(move |application| {
        if let Some(window) = application.active_window() {
            window.present();
            return;
        }
        build_main_window(application, Rc::clone(&activation_state));
    });

    let shutdown_state = Rc::clone(&server_state);
    application.connect_shutdown(move |_| shutdown_state.borrow_mut().shutdown());

    Ok(application.run())
}

fn runtime_directory() -> PathBuf {
    let root = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::temp_dir().join(format!("kulkan-{}", unsafe { libc::geteuid() }))
        });
    root.join(APPLICATION_ID)
}

struct ServerState {
    executable: PathBuf,
    socket_path: PathBuf,
    process: Option<ServerProcess>,
}

impl ServerState {
    fn start(&mut self) -> Result<(), String> {
        if let Some(process) = self.process.as_mut() {
            if process.is_running().map_err(|error| error.to_string())? {
                return process.ping().map_err(|error| error.to_string());
            }
            self.process = None;
        }
        self.process = Some(
            ServerProcess::start(&self.executable, &self.socket_path, Duration::from_secs(2))
                .map_err(|error| error.to_string())?,
        );
        Ok(())
    }

    fn shutdown(&mut self) {
        if let Some(mut process) = self.process.take() {
            if let Err(error) = process.shutdown() {
                eprintln!("Kulkán server shutdown failed: {error}");
            }
        }
    }
}

fn build_main_window(application: &gtk::Application, server_state: Rc<RefCell<ServerState>>) {
    let window = gtk::ApplicationWindow::builder()
        .application(application)
        .title("Kulkán")
        .default_width(720)
        .default_height(460)
        .build();
    window.set_icon_name(Some(APPLICATION_ID));

    let content = gtk::Box::new(gtk::Orientation::Vertical, 16);
    content.set_margin_top(32);
    content.set_margin_bottom(32);
    content.set_margin_start(32);
    content.set_margin_end(32);

    let logo = gtk::Image::from_icon_name(APPLICATION_ID);
    logo.set_pixel_size(144);
    logo.set_halign(gtk::Align::Center);
    content.append(&logo);

    let title = gtk::Label::new(Some("Kulkán"));
    title.add_css_class("title-1");
    content.append(&title);

    let status = gtk::Label::new(None);
    status.set_wrap(true);
    status.set_selectable(true);
    content.append(&status);

    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    actions.set_halign(gtk::Align::Center);
    let retry = gtk::Button::with_label("Retry");
    let quit = gtk::Button::with_label("Quit");
    actions.append(&retry);
    actions.append(&quit);
    content.append(&actions);
    window.set_child(Some(&content));

    match server_state.borrow_mut().start() {
        Ok(()) => {
            status.set_text("Server ready. Kulkán is ready to use.");
            retry.set_sensitive(false);
        }
        Err(error) => {
            status.set_text(&format!("Server unavailable: {error}"));
            retry.set_sensitive(true);
        }
    }

    let retry_status = status.clone();
    let retry_state = Rc::clone(&server_state);
    let retry_button = retry.clone();
    retry.connect_clicked(move |_| match retry_state.borrow_mut().start() {
        Ok(()) => {
            retry_status.set_text("Server ready. Kulkán is ready to use.");
            retry_button.set_sensitive(false);
        }
        Err(error) => {
            retry_status.set_text(&format!("Server unavailable: {error}"));
            retry_button.set_sensitive(true);
        }
    });

    let quit_application = application.clone();
    quit.connect_clicked(move |_| quit_application.quit());
    window.present();
}

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Duration;

use gtk::gio;
use gtk::glib;
use gtk::prelude::*;
use kulkan_library::{CatalogPage, LibraryEntry, LibraryState, ScanSummary, present_catalog_entry};
use kulkan_runtime::{ClientInstance, ServerClient, ServerProcess, activate_existing_client};

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

    fn client(&self) -> Option<ServerClient> {
        self.process.as_ref().map(ServerProcess::client)
    }
}

fn build_main_window(application: &gtk::Application, server_state: Rc<RefCell<ServerState>>) {
    let window = gtk::ApplicationWindow::builder()
        .application(application)
        .title("Kulkán")
        .default_width(1120)
        .default_height(720)
        .build();
    window.set_icon_name(Some(APPLICATION_ID));

    let content = gtk::Box::new(gtk::Orientation::Horizontal, 16);
    content.set_margin_top(16);
    content.set_margin_bottom(16);
    content.set_margin_start(16);
    content.set_margin_end(16);

    let library_sidebar = gtk::Box::new(gtk::Orientation::Vertical, 8);
    library_sidebar.set_size_request(240, -1);
    let logo = gtk::Image::from_icon_name(APPLICATION_ID);
    logo.set_pixel_size(48);
    logo.set_halign(gtk::Align::Start);
    library_sidebar.append(&logo);
    let title = gtk::Label::new(Some("Library"));
    title.add_css_class("title-1");
    title.set_halign(gtk::Align::Start);
    library_sidebar.append(&title);
    let roots_label = gtk::Label::new(Some("Library folders"));
    roots_label.set_halign(gtk::Align::Start);
    library_sidebar.append(&roots_label);
    let roots_list = gtk::ListBox::new();
    roots_list.set_selection_mode(gtk::SelectionMode::Single);
    let roots_scroll = gtk::ScrolledWindow::builder()
        .child(&roots_list)
        .vexpand(true)
        .build();
    library_sidebar.append(&roots_scroll);
    let root_actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let add_root = gtk::Button::with_label("Add Folder…");
    let remove_root = gtk::Button::with_label("Remove");
    remove_root.set_sensitive(false);
    root_actions.append(&add_root);
    root_actions.append(&remove_root);
    library_sidebar.append(&root_actions);
    let scan_button = gtk::Button::with_label("Scan Library");
    library_sidebar.append(&scan_button);

    let main_content = gtk::Box::new(gtk::Orientation::Vertical, 8);
    let status = gtk::Label::new(None);
    status.set_wrap(true);
    status.set_selectable(true);
    status.set_halign(gtk::Align::Start);
    main_content.append(&status);
    let catalog_label = gtk::Label::new(Some("Tracks"));
    catalog_label.set_halign(gtk::Align::Start);
    main_content.append(&catalog_label);
    let catalog_list = gtk::ListBox::new();
    catalog_list.set_selection_mode(gtk::SelectionMode::Single);
    let catalog_scroll = gtk::ScrolledWindow::builder()
        .child(&catalog_list)
        .vexpand(true)
        .hexpand(true)
        .build();
    main_content.append(&catalog_scroll);
    let load_more = gtk::Button::with_label("Load More Tracks");
    load_more.set_visible(false);
    main_content.append(&load_more);

    let queue_placeholder = gtk::Box::new(gtk::Orientation::Vertical, 8);
    queue_placeholder.set_size_request(220, -1);
    let queue_title = gtk::Label::new(Some("Queue"));
    queue_title.add_css_class("title-2");
    queue_title.set_halign(gtk::Align::Start);
    queue_placeholder.append(&queue_title);
    let queue_status = gtk::Label::new(Some("Select tracks from your library after a scan."));
    queue_status.set_wrap(true);
    queue_status.set_halign(gtk::Align::Start);
    queue_placeholder.append(&queue_status);

    content.append(&library_sidebar);
    content.append(&main_content);
    content.append(&queue_placeholder);

    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    actions.set_halign(gtk::Align::Center);
    let retry = gtk::Button::with_label("Retry");
    let quit = gtk::Button::with_label("Quit");
    actions.append(&retry);
    actions.append(&quit);
    let shell = gtk::Box::new(gtk::Orientation::Vertical, 8);
    shell.append(&content);
    shell.append(&actions);
    window.set_child(Some(&shell));

    let roots_model = Rc::new(RefCell::new(Vec::<PathBuf>::new()));
    let (sender, receiver) = mpsc::channel();
    let catalog_paging = Rc::new(RefCell::new(None::<(ServerClient, usize)>));
    install_library_events(
        receiver,
        LibraryUi {
            roots_list: roots_list.clone(),
            catalog_list: catalog_list.clone(),
            roots_model: Rc::clone(&roots_model),
            catalog_paging: Rc::clone(&catalog_paging),
            status: status.clone(),
            scan_button: scan_button.clone(),
            load_more: load_more.clone(),
        },
    );
    let remove_for_selection = remove_root.clone();
    let selection_state = Rc::clone(&server_state);
    roots_list.connect_row_selected(move |_, row| {
        remove_for_selection
            .set_sensitive(row.is_some() && selection_state.borrow().client().is_some());
    });

    let chooser_state = Rc::clone(&server_state);
    let chooser_sender = sender.clone();
    let chooser_status = status.clone();
    let chooser_parent = window.clone();
    add_root.connect_clicked(move |_| {
        let chooser = gtk::FileChooserNative::new(
            Some("Choose Library Folder"),
            Some(&chooser_parent),
            gtk::FileChooserAction::SelectFolder,
            Some("Select Folder"),
            Some("Cancel"),
        );
        let state = Rc::clone(&chooser_state);
        let sender = chooser_sender.clone();
        let status = chooser_status.clone();
        chooser.connect_response(move |chooser, response| {
            if response == gtk::ResponseType::Accept {
                match chooser.file().and_then(|file| file.path()) {
                    Some(path) => match state.borrow().client() {
                        Some(client) => {
                            send_root_operation(client, path, sender.clone(), false);
                            status.set_text("Adding folder…");
                        }
                        None => status.set_text("Server unavailable. Select Retry."),
                    },
                    None => {
                        status.set_text("The selected folder is not available as a local path.")
                    }
                }
            }
            chooser.hide();
        });
        chooser.show();
    });

    let remove_state = Rc::clone(&server_state);
    let remove_roots = Rc::clone(&roots_model);
    let remove_sender = sender.clone();
    let remove_status = status.clone();
    let root_list_for_remove = roots_list.clone();
    remove_root.connect_clicked(move |_| {
        let Some(row) = root_list_for_remove.selected_row() else {
            return;
        };
        let Some(path) = remove_roots.borrow().get(row.index() as usize).cloned() else {
            return;
        };
        let Some(client) = remove_state.borrow().client() else {
            remove_status.set_text("Server unavailable. Select Retry.");
            return;
        };
        send_root_operation(client, path, remove_sender.clone(), true);
        remove_status.set_text("Removing folder…");
    });

    let scan_state = Rc::clone(&server_state);
    let scan_sender = sender.clone();
    let scan_status = status.clone();
    let scan_button_for_action = scan_button.clone();
    let scan_load_more = load_more.clone();
    let scan_paging = Rc::clone(&catalog_paging);
    scan_button.connect_clicked(move |_| {
        let Some(client) = scan_state.borrow().client() else {
            scan_status.set_text("Server unavailable. Select Retry.");
            return;
        };
        scan_button_for_action.set_sensitive(false);
        scan_load_more.set_visible(false);
        scan_load_more.set_sensitive(false);
        scan_paging.borrow_mut().take();
        scan_status.set_text("Scanning selected folders…");
        let sender = scan_sender.clone();
        thread::spawn(move || {
            let _ = sender.send(UiEvent::Catalog(read_scanned_catalog(&client)));
        });
    });

    let paging_for_button = Rc::clone(&catalog_paging);
    let paging_sender = sender.clone();
    let load_more_status = status.clone();
    let load_more_button = load_more.clone();
    load_more.connect_clicked(move |_| {
        let Some((client, offset)) = paging_for_button.borrow().clone() else {
            return;
        };
        load_more_button.set_sensitive(false);
        load_more_status.set_text("Loading more tracks…");
        let sender = paging_sender.clone();
        thread::spawn(move || {
            let result = client
                .catalog_page(offset, 32)
                .map_err(|error| error.to_string());
            let _ = sender.send(UiEvent::CatalogPage(result));
        });
    });

    let startup_result = { server_state.borrow_mut().start() };
    match startup_result {
        Ok(()) => {
            status.set_text("Server ready. Add a library folder, then scan it.");
            retry.set_sensitive(false);
            set_server_controls_sensitive(
                &add_root,
                &remove_root,
                &scan_button,
                roots_list.selected_row().is_some(),
                true,
            );
            if let Some(client) = server_state.borrow().client() {
                let sender = sender.clone();
                thread::spawn(move || {
                    let _ = sender.send(UiEvent::Roots(
                        client.library_state().map_err(|error| error.to_string()),
                    ));
                });
            }
        }
        Err(error) => {
            status.set_text(&format!("Server unavailable: {error}"));
            retry.set_sensitive(true);
            set_server_controls_sensitive(&add_root, &remove_root, &scan_button, false, false);
        }
    }

    let retry_status = status.clone();
    let retry_state = Rc::clone(&server_state);
    let retry_button = retry.clone();
    let retry_sender = sender;
    let retry_add = add_root.clone();
    let retry_remove = remove_root.clone();
    let retry_scan = scan_button.clone();
    retry.connect_clicked(move |_| {
        let retry_result = { retry_state.borrow_mut().start() };
        match retry_result {
            Ok(()) => {
                retry_status.set_text("Server ready. Add a library folder, then scan it.");
                retry_button.set_sensitive(false);
                set_server_controls_sensitive(
                    &retry_add,
                    &retry_remove,
                    &retry_scan,
                    roots_list.selected_row().is_some(),
                    true,
                );
                if let Some(client) = retry_state.borrow().client() {
                    let sender = retry_sender.clone();
                    thread::spawn(move || {
                        let _ = sender.send(UiEvent::Roots(
                            client.library_state().map_err(|error| error.to_string()),
                        ));
                    });
                }
            }
            Err(error) => {
                retry_status.set_text(&format!("Server unavailable: {error}"));
                retry_button.set_sensitive(true);
                set_server_controls_sensitive(&retry_add, &retry_remove, &retry_scan, false, false);
            }
        }
    });

    let quit_application = application.clone();
    quit.connect_clicked(move |_| quit_application.quit());
    window.present();
}

enum UiEvent {
    Roots(Result<LibraryState, String>),
    Catalog(Result<(ScanSummary, CatalogPage, ServerClient), String>),
    CatalogPage(Result<CatalogPage, String>),
}

struct LibraryUi {
    roots_list: gtk::ListBox,
    catalog_list: gtk::ListBox,
    roots_model: Rc<RefCell<Vec<PathBuf>>>,
    catalog_paging: Rc<RefCell<Option<(ServerClient, usize)>>>,
    status: gtk::Label,
    scan_button: gtk::Button,
    load_more: gtk::Button,
}

fn install_library_events(receiver: Receiver<UiEvent>, ui: LibraryUi) {
    let LibraryUi {
        roots_list,
        catalog_list,
        roots_model,
        catalog_paging,
        status,
        scan_button,
        load_more,
    } = ui;
    glib::timeout_add_local(Duration::from_millis(40), move || {
        while let Ok(event) = receiver.try_recv() {
            match event {
                UiEvent::Roots(Ok(state)) => {
                    *roots_model.borrow_mut() = state.selected_roots.clone();
                    replace_root_rows(&roots_list, &state.selected_roots);
                    status
                        .set_text("Library folders updated. Scan Library to refresh the catalog.");
                }
                UiEvent::Roots(Err(error)) => {
                    status.set_text(&format!("Library operation failed: {error}"));
                }
                UiEvent::Catalog(Ok((summary, page, client))) => {
                    replace_catalog_rows(&catalog_list, &page.entries);
                    scan_button.set_sensitive(true);
                    status.set_text(&scan_status_text(&summary));
                    *catalog_paging.borrow_mut() = page.next_offset.map(|offset| (client, offset));
                    load_more.set_visible(page.next_offset.is_some());
                    load_more.set_sensitive(page.next_offset.is_some());
                }
                UiEvent::Catalog(Err(error)) => {
                    scan_button.set_sensitive(true);
                    status.set_text(&format!("Library scan failed: {error}"));
                }
                UiEvent::CatalogPage(Ok(page)) => {
                    append_catalog_rows(&catalog_list, &page.entries);
                    let client = catalog_paging
                        .borrow()
                        .as_ref()
                        .map(|(client, _)| client.clone());
                    *catalog_paging.borrow_mut() = client.zip(page.next_offset);
                    load_more.set_visible(page.next_offset.is_some());
                    load_more.set_sensitive(page.next_offset.is_some());
                    status.set_text("More catalog entries loaded.");
                }
                UiEvent::CatalogPage(Err(error)) => {
                    load_more.set_sensitive(true);
                    status.set_text(&format!("Could not load more tracks: {error}"));
                }
            }
        }
        glib::ControlFlow::Continue
    });
}

fn replace_root_rows(list: &gtk::ListBox, roots: &[PathBuf]) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }
    for root in roots {
        let label = gtk::Label::new(Some(&root.display().to_string()));
        label.set_halign(gtk::Align::Start);
        label.set_wrap(true);
        let row = gtk::ListBoxRow::new();
        row.set_child(Some(&label));
        list.append(&row);
    }
}

fn replace_catalog_rows(list: &gtk::ListBox, entries: &[LibraryEntry]) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }
    append_catalog_rows(list, entries);
}

fn append_catalog_rows(list: &gtk::ListBox, entries: &[LibraryEntry]) {
    for entry in entries {
        list.append(&render_catalog_row(entry));
    }
}

fn render_catalog_row(entry: &LibraryEntry) -> gtk::ListBoxRow {
    let view = present_catalog_entry(entry);
    let row = gtk::ListBoxRow::new();
    row.set_sensitive(view.selectable);
    row.set_selectable(view.selectable);
    row.set_activatable(view.selectable);

    let content = gtk::Box::new(gtk::Orientation::Vertical, 4);
    let title = view.title.as_deref().unwrap_or("(No title tag)");
    let title = if view.active {
        title.to_owned()
    } else {
        format!("Unavailable — {title}")
    };
    let title_label = gtk::Label::new(Some(&title));
    title_label.set_halign(gtk::Align::Start);
    title_label.add_css_class("heading");
    content.append(&title_label);

    let artist = if view.artists.is_empty() {
        "(No artist tag)".to_owned()
    } else {
        view.artists.join(", ")
    };
    let release = view.release_title.as_deref().unwrap_or("(No release tag)");
    let track_number = view
        .track_number
        .map(|number| format!(" · Track {number}"))
        .unwrap_or_default();
    let year = view
        .release_year
        .map(|year| format!(" · {year}"))
        .unwrap_or_default();
    let detail = gtk::Label::new(Some(&format!("{artist} · {release}{year}{track_number}")));
    detail.set_halign(gtk::Align::Start);
    detail.set_wrap(true);
    content.append(&detail);

    let genres = if view.genres.is_empty() {
        "(No genre tag)".to_owned()
    } else {
        view.genres.join(" · ")
    };
    let duration = view
        .duration
        .map(|duration| format!("{}:{:02}", duration.as_secs() / 60, duration.as_secs() % 60))
        .unwrap_or_else(|| "Duration unavailable".to_owned());
    let metadata = gtk::Label::new(Some(&format!(
        "{genres} · {duration}\n{}",
        view.location.display()
    )));
    metadata.set_halign(gtk::Align::Start);
    metadata.set_wrap(true);
    metadata.add_css_class("dim-label");
    content.append(&metadata);

    row.set_child(Some(&content));
    row
}

fn send_root_operation(client: ServerClient, path: PathBuf, sender: Sender<UiEvent>, remove: bool) {
    thread::spawn(move || {
        let result = if remove {
            client.remove_library_root(&path)
        } else {
            client.add_library_root(&path)
        }
        .map_err(|error| error.to_string());
        let _ = sender.send(UiEvent::Roots(result));
    });
}

fn read_scanned_catalog(
    client: &ServerClient,
) -> Result<(ScanSummary, CatalogPage, ServerClient), String> {
    let summary = client.scan_library().map_err(|error| error.to_string())?;
    let page = client
        .catalog_page(0, 32)
        .map_err(|error| error.to_string())?;
    Ok((summary, page, client.clone()))
}

fn scan_status_text(summary: &ScanSummary) -> String {
    let mut text = format!(
        "Scan complete: {} active, {} inactive; {} incomplete area(s), {} media error(s).",
        summary.active_entries,
        summary.inactive_entries,
        summary.incomplete_paths.len(),
        summary.media_errors.len()
    );
    for path in &summary.incomplete_paths {
        text.push_str(&format!("\nIncomplete: {}", path.display()));
    }
    for error in &summary.media_errors {
        text.push_str(&format!(
            "\nMedia error: {} — {}",
            error.path.display(),
            error.message
        ));
    }
    text
}

fn set_server_controls_sensitive(
    add_root: &gtk::Button,
    remove_root: &gtk::Button,
    scan_button: &gtk::Button,
    root_is_selected: bool,
    sensitive: bool,
) {
    add_root.set_sensitive(sensitive);
    scan_button.set_sensitive(sensitive);
    remove_root.set_sensitive(sensitive && root_is_selected);
}

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Duration;

use gtk::gio;
use gtk::glib;
use gtk::prelude::*;
use kulkan_library::{CatalogPage, LibraryEntry, LibraryState, ScanSummary, present_catalog_entry};
use kulkan_runtime::{
    ClientInstance, PlaybackSnapshot, PlaybackStatus, QueueEntry, QueueSnapshot, RepeatMode,
    ServerClient, ServerProcess, activate_existing_client,
};

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
    let library_state_path = data_directory().join("library.json");

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
        library_state_path,
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

fn data_directory() -> PathBuf {
    let root = std::env::var_os("XDG_DATA_HOME")
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .filter(|home| !home.is_empty())
                .map(|home| PathBuf::from(home).join(".local/share"))
        })
        .unwrap_or_else(|| {
            std::env::temp_dir().join(format!("kulkan-data-{}", unsafe { libc::geteuid() }))
        });
    if std::env::var_os("FLATPAK_ID").as_deref() == Some(std::ffi::OsStr::new(APPLICATION_ID)) {
        root
    } else {
        root.join(APPLICATION_ID)
    }
}

struct ServerState {
    executable: PathBuf,
    socket_path: PathBuf,
    library_state_path: PathBuf,
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
            ServerProcess::start_with_library_state(
                &self.executable,
                &self.socket_path,
                &self.library_state_path,
                Duration::from_secs(2),
            )
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

    let current_track = gtk::Label::new(Some("Nothing is playing."));
    current_track.set_halign(gtk::Align::Start);
    current_track.set_ellipsize(gtk::pango::EllipsizeMode::End);
    main_content.append(&current_track);
    let playback_position = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 1.0, 1.0);
    playback_position.set_hexpand(true);
    playback_position.set_draw_value(false);
    playback_position.set_sensitive(false);
    playback_position.set_tooltip_text(Some("Click the timeline to seek."));
    main_content.append(&playback_position);
    let playback_controls = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let previous_track = gtk::Button::with_label("Previous");
    let stop_playback = gtk::Button::with_label("Stop");
    let play_pause = gtk::Button::with_label("Play/Pause");
    let next_track = gtk::Button::with_label("Next");
    previous_track.set_sensitive(false);
    stop_playback.set_sensitive(false);
    play_pause.set_sensitive(false);
    next_track.set_sensitive(false);
    for button in [&previous_track, &stop_playback, &play_pause, &next_track] {
        playback_controls.append(button);
    }
    main_content.append(&playback_controls);

    let queue_placeholder = gtk::Box::new(gtk::Orientation::Vertical, 8);
    queue_placeholder.set_size_request(260, -1);
    let queue_title = gtk::Label::new(Some("Queue"));
    queue_title.add_css_class("title-2");
    queue_title.set_halign(gtk::Align::Start);
    queue_placeholder.append(&queue_title);
    let queue_list = gtk::ListBox::new();
    queue_list.set_selection_mode(gtk::SelectionMode::Single);
    let queue_scroll = gtk::ScrolledWindow::builder()
        .child(&queue_list)
        .vexpand(true)
        .build();
    queue_placeholder.append(&queue_scroll);
    let queue_repeat = gtk::DropDown::from_strings(&["Repeat Off", "Repeat One", "Repeat All"]);
    queue_placeholder.append(&queue_repeat);
    let queue_shuffle = gtk::CheckButton::with_label("Shuffle");
    queue_placeholder.append(&queue_shuffle);
    let queue_actions = gtk::Grid::new();
    queue_actions.set_column_spacing(6);
    queue_actions.set_row_spacing(6);
    let add_selected = gtk::Button::with_label("Add Selected");
    let add_all = gtk::Button::with_label("Add All Tracks");
    let play_queue_entry = gtk::Button::with_label("Play Queue Entry");
    let skip_queue_entry = next_track.clone();
    let move_queue_up = gtk::Button::with_label("Move Up");
    let move_queue_down = gtk::Button::with_label("Move Down");
    let remove_queue_entry = gtk::Button::with_label("Remove");
    let clear_queue = gtk::Button::with_label("Clear Queue");
    for button in [
        &add_selected,
        &add_all,
        &play_queue_entry,
        &move_queue_up,
        &move_queue_down,
        &remove_queue_entry,
        &clear_queue,
    ] {
        button.set_sensitive(false);
    }
    queue_repeat.set_sensitive(false);
    queue_shuffle.set_sensitive(false);
    for (column, row, button) in [
        (0, 0, &add_selected),
        (1, 0, &add_all),
        (0, 1, &play_queue_entry),
        (1, 1, &move_queue_up),
        (0, 2, &move_queue_down),
        (1, 2, &remove_queue_entry),
        (0, 3, &clear_queue),
    ] {
        queue_actions.attach(button, column, row, 1, 1);
    }
    queue_placeholder.append(&queue_actions);

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
    let catalog_entries = Rc::new(RefCell::new(Vec::<LibraryEntry>::new()));
    let queue_entries = Rc::new(RefCell::new(Vec::<QueueEntry>::new()));
    let displayed_queue_current = Rc::new(Cell::new(None));
    let syncing_queue_controls = Rc::new(Cell::new(false));
    let poll_playback = Arc::new(AtomicBool::new(false));
    let (sender, receiver) = mpsc::channel();
    let catalog_paging = Rc::new(RefCell::new(None::<(ServerClient, usize)>));
    let queue_ui = QueueUi {
        list: queue_list.clone(),
        entries: Rc::clone(&queue_entries),
        current_entry_id: Rc::clone(&displayed_queue_current),
        syncing_controls: Rc::clone(&syncing_queue_controls),
        server_available: Rc::new(Cell::new(false)),
        repeat: queue_repeat.clone(),
        shuffle: queue_shuffle.clone(),
        previous: previous_track.clone(),
        toggle: play_pause.clone(),
        play: play_queue_entry.clone(),
        skip: skip_queue_entry.clone(),
        move_up: move_queue_up.clone(),
        move_down: move_queue_down.clone(),
        remove: remove_queue_entry.clone(),
        clear: clear_queue.clone(),
    };
    install_library_events(
        receiver,
        LibraryUi {
            roots_list: roots_list.clone(),
            catalog_list: catalog_list.clone(),
            roots_model: Rc::clone(&roots_model),
            catalog_entries: Rc::clone(&catalog_entries),
            catalog_paging: Rc::clone(&catalog_paging),
            status: status.clone(),
            scan_button: scan_button.clone(),
            load_more: load_more.clone(),
            queue: queue_ui.clone(),
            playback: PlaybackUi {
                current_track: current_track.clone(),
                position: playback_position.clone(),
                toggle: play_pause.clone(),
                stop: stop_playback.clone(),
            },
            poll_playback: Arc::clone(&poll_playback),
        },
    );
    let selection_queue_ui = queue_ui.clone();
    queue_list.connect_row_selected(move |_, _| {
        selection_queue_ui.update_actions();
    });
    let selection_model = Rc::clone(&catalog_entries);
    let add_for_selection = add_selected.clone();
    let selection_server = Rc::clone(&server_state);
    catalog_list.set_selection_mode(gtk::SelectionMode::Multiple);
    let selection_click = gtk::GestureClick::new();
    selection_click.set_button(1);
    selection_click.set_propagation_phase(gtk::PropagationPhase::Capture);
    let selection_list = catalog_list.clone();
    selection_click.connect_pressed(move |gesture, _, _, y| {
        if let Some(row) = selection_list.row_at_y(y as i32)
            && row.is_selected()
        {
            selection_list.unselect_row(&row);
            gesture.set_state(gtk::EventSequenceState::Claimed);
        }
    });
    catalog_list.add_controller(selection_click);
    catalog_list.connect_row_selected(move |list, _| {
        let can_queue = list.selected_rows().iter().any(|row| {
            selection_model
                .borrow()
                .get(row.index() as usize)
                .is_some_and(|entry| entry.active)
        });
        let can_queue = can_queue && selection_server.borrow().client().is_some();
        add_for_selection.set_sensitive(can_queue);
    });

    let play_state = Rc::clone(&server_state);
    let play_model = Rc::clone(&catalog_entries);
    let play_list = catalog_list.clone();
    let play_status = status.clone();
    let play_sender = sender.clone();
    play_pause.connect_clicked(move |_| {
        let tracks = selected_active_tracks(&play_list, &play_model);
        let Some(client) = play_state.borrow().client() else {
            play_status.set_text("Server unavailable. Select Retry.");
            return;
        };
        send_playback_operation(client, PlaybackRequest::Toggle(tracks), play_sender.clone());
    });

    let add_state = Rc::clone(&server_state);
    let add_model = Rc::clone(&catalog_entries);
    let add_list = catalog_list.clone();
    let add_status = status.clone();
    let add_sender = sender.clone();
    add_selected.connect_clicked(move |_| {
        let tracks = selected_active_tracks(&add_list, &add_model);
        if tracks.is_empty() {
            return;
        }
        let Some(client) = add_state.borrow().client() else {
            add_status.set_text("Server unavailable. Select Retry.");
            return;
        };
        send_playback_operation(client, PlaybackRequest::Append(tracks), add_sender.clone());
    });

    let all_state = Rc::clone(&server_state);
    let all_status = status.clone();
    let all_sender = sender.clone();
    add_all.connect_clicked(move |_| {
        let Some(client) = all_state.borrow().client() else {
            all_status.set_text("Server unavailable. Select Retry.");
            return;
        };
        all_status.set_text("Adding all active tracks to the Queue…");
        send_playback_operation(
            client,
            PlaybackRequest::AppendAllCatalog,
            all_sender.clone(),
        );
    });

    let previous_state = Rc::clone(&server_state);
    let previous_sender = sender.clone();
    previous_track.connect_clicked(move |_| {
        if let Some(client) = previous_state.borrow().client() {
            send_playback_operation(client, PlaybackRequest::Previous, previous_sender.clone());
        }
    });

    let seek_state = Rc::clone(&server_state);
    let seek_scale = playback_position.clone();
    let seek_status = status.clone();
    let seek_sender = sender.clone();
    let seek_gesture = gtk::GestureClick::new();
    seek_gesture.set_button(1);
    seek_gesture.set_propagation_phase(gtk::PropagationPhase::Capture);
    seek_gesture.connect_pressed(|gesture, _, _, _| {
        gesture.set_state(gtk::EventSequenceState::Claimed);
    });
    seek_gesture.connect_released(move |gesture, _, x, _| {
        let width = seek_scale.width() as f64;
        let duration_ms = seek_scale.adjustment().upper();
        if width <= 0.0 || duration_ms <= 0.0 {
            return;
        }
        let fraction = (x / width).clamp(0.0, 1.0);
        let fraction = if seek_scale.direction() == gtk::TextDirection::Rtl {
            1.0 - fraction
        } else {
            fraction
        };
        let position_ms = (duration_ms * fraction).round() as u64;
        seek_scale.set_value(position_ms as f64);
        gesture.set_state(gtk::EventSequenceState::Claimed);
        if let Some(client) = seek_state.borrow().client() {
            send_playback_operation(
                client,
                PlaybackRequest::Seek(position_ms),
                seek_sender.clone(),
            );
        } else {
            seek_status.set_text("Server unavailable. Select Retry.");
        }
    });
    playback_position.add_controller(seek_gesture);
    let keyboard_seek_state = Rc::clone(&server_state);
    let keyboard_seek_sender = sender.clone();
    playback_position.connect_change_value(move |_, _, value| {
        if let Some(client) = keyboard_seek_state.borrow().client() {
            send_playback_operation(
                client,
                PlaybackRequest::Seek(value.max(0.0).round() as u64),
                keyboard_seek_sender.clone(),
            );
        }
        glib::Propagation::Proceed
    });

    let stop_state = Rc::clone(&server_state);
    let stop_status = status.clone();
    let stop_sender = sender.clone();
    stop_playback.connect_clicked(move |_| {
        if let Some(client) = stop_state.borrow().client() {
            send_playback_operation(client, PlaybackRequest::Stop, stop_sender.clone());
        } else {
            stop_status.set_text("Server unavailable. Select Retry.");
        }
    });

    let play_queue_state = Rc::clone(&server_state);
    let play_queue_model = Rc::clone(&queue_entries);
    let play_queue_list = queue_list.clone();
    let play_queue_sender = sender.clone();
    play_queue_entry.connect_clicked(move |_| {
        let Some(row) = play_queue_list.selected_row() else {
            return;
        };
        let Some(entry) = play_queue_model.borrow().get(row.index() as usize).cloned() else {
            return;
        };
        if let Some(client) = play_queue_state.borrow().client() {
            send_playback_operation(
                client,
                PlaybackRequest::PlayQueueEntry(entry.id),
                play_queue_sender.clone(),
            );
        }
    });

    let skip_state = Rc::clone(&server_state);
    let skip_sender = sender.clone();
    skip_queue_entry.connect_clicked(move |_| {
        if let Some(client) = skip_state.borrow().client() {
            send_playback_operation(client, PlaybackRequest::Skip, skip_sender.clone());
        }
    });

    let reorder_state = Rc::clone(&server_state);
    let reorder_model = Rc::clone(&queue_entries);
    let reorder_list = queue_list.clone();
    let reorder_sender = sender.clone();
    move_queue_up.connect_clicked(move |_| {
        let Some(row) = reorder_list.selected_row() else {
            return;
        };
        let position = row.index() as usize;
        if position == 0 {
            return;
        }
        let Some(entry) = reorder_model.borrow().get(position).cloned() else {
            return;
        };
        if let Some(client) = reorder_state.borrow().client() {
            send_playback_operation(
                client,
                PlaybackRequest::Reorder(entry.id, position - 1),
                reorder_sender.clone(),
            );
        }
    });

    let reorder_state = Rc::clone(&server_state);
    let reorder_model = Rc::clone(&queue_entries);
    let reorder_list = queue_list.clone();
    let reorder_sender = sender.clone();
    move_queue_down.connect_clicked(move |_| {
        let Some(row) = reorder_list.selected_row() else {
            return;
        };
        let position = row.index() as usize;
        let Some(entry) = reorder_model.borrow().get(position).cloned() else {
            return;
        };
        if position + 1 >= reorder_model.borrow().len() {
            return;
        }
        if let Some(client) = reorder_state.borrow().client() {
            send_playback_operation(
                client,
                PlaybackRequest::Reorder(entry.id, position + 1),
                reorder_sender.clone(),
            );
        }
    });

    let remove_queue_state = Rc::clone(&server_state);
    let remove_queue_model = Rc::clone(&queue_entries);
    let remove_queue_list = queue_list.clone();
    let remove_queue_sender = sender.clone();
    remove_queue_entry.connect_clicked(move |_| {
        let Some(row) = remove_queue_list.selected_row() else {
            return;
        };
        let Some(entry) = remove_queue_model
            .borrow()
            .get(row.index() as usize)
            .cloned()
        else {
            return;
        };
        if let Some(client) = remove_queue_state.borrow().client() {
            send_playback_operation(
                client,
                PlaybackRequest::Remove(entry.id),
                remove_queue_sender.clone(),
            );
        }
    });

    let clear_queue_state = Rc::clone(&server_state);
    let clear_queue_sender = sender.clone();
    clear_queue.connect_clicked(move |_| {
        if let Some(client) = clear_queue_state.borrow().client() {
            send_playback_operation(client, PlaybackRequest::Clear, clear_queue_sender.clone());
        }
    });

    let repeat_state = Rc::clone(&server_state);
    let repeat_syncing = Rc::clone(&syncing_queue_controls);
    let repeat_sender = sender.clone();
    queue_repeat.connect_selected_notify(move |repeat| {
        if repeat_syncing.get() {
            return;
        }
        let mode = match repeat.selected() {
            1 => RepeatMode::One,
            2 => RepeatMode::All,
            _ => RepeatMode::Off,
        };
        if let Some(client) = repeat_state.borrow().client() {
            send_playback_operation(client, PlaybackRequest::Repeat(mode), repeat_sender.clone());
        }
    });

    let shuffle_state = Rc::clone(&server_state);
    let shuffle_syncing = Rc::clone(&syncing_queue_controls);
    let shuffle_sender = sender.clone();
    queue_shuffle.connect_toggled(move |shuffle| {
        if shuffle_syncing.get() {
            return;
        }
        if let Some(client) = shuffle_state.borrow().client() {
            send_playback_operation(
                client,
                PlaybackRequest::Shuffle(shuffle.is_active()),
                shuffle_sender.clone(),
            );
        }
    });

    install_playback_poll(
        Rc::clone(&server_state),
        sender.clone(),
        Arc::clone(&poll_playback),
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

    let startup_add_all = add_all.clone();
    let startup_repeat = queue_repeat.clone();
    let startup_shuffle = queue_shuffle.clone();
    let startup_queue_ui = queue_ui.clone();
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
            startup_add_all.set_sensitive(true);
            startup_repeat.set_sensitive(true);
            startup_shuffle.set_sensitive(true);
            startup_queue_ui.set_available(true);
            if let Some(client) = server_state.borrow().client() {
                let sender = sender.clone();
                thread::spawn(move || {
                    let _ = sender.send(UiEvent::RestoredLibrary(read_restored_library(&client)));
                    let _ = sender.send(UiEvent::Playback {
                        state: client.queue_state().map_err(|error| error.to_string()),
                        command_error: None,
                    });
                });
            }
        }
        Err(error) => {
            status.set_text(&format!("Server unavailable: {error}"));
            retry.set_sensitive(true);
            set_server_controls_sensitive(&add_root, &remove_root, &scan_button, false, false);
            startup_add_all.set_sensitive(false);
            startup_repeat.set_sensitive(false);
            startup_shuffle.set_sensitive(false);
            startup_queue_ui.set_available(false);
        }
    }

    let retry_status = status.clone();
    let retry_state = Rc::clone(&server_state);
    let retry_button = retry.clone();
    let retry_sender = sender;
    let retry_add = add_root.clone();
    let retry_remove = remove_root.clone();
    let retry_scan = scan_button.clone();
    let retry_add_all = add_all.clone();
    let retry_repeat = queue_repeat.clone();
    let retry_shuffle = queue_shuffle.clone();
    let retry_queue_ui = queue_ui.clone();
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
                retry_add_all.set_sensitive(true);
                retry_repeat.set_sensitive(true);
                retry_shuffle.set_sensitive(true);
                retry_queue_ui.set_available(true);
                if let Some(client) = retry_state.borrow().client() {
                    let sender = retry_sender.clone();
                    thread::spawn(move || {
                        let _ =
                            sender.send(UiEvent::RestoredLibrary(read_restored_library(&client)));
                        let _ = sender.send(UiEvent::Playback {
                            state: client.queue_state().map_err(|error| error.to_string()),
                            command_error: None,
                        });
                    });
                }
            }
            Err(error) => {
                retry_status.set_text(&format!("Server unavailable: {error}"));
                retry_button.set_sensitive(true);
                set_server_controls_sensitive(&retry_add, &retry_remove, &retry_scan, false, false);
                retry_add_all.set_sensitive(false);
                retry_repeat.set_sensitive(false);
                retry_shuffle.set_sensitive(false);
                retry_queue_ui.set_available(false);
            }
        }
    });

    let quit_application = application.clone();
    quit.connect_clicked(move |_| quit_application.quit());
    window.present();
}

enum UiEvent {
    Roots(Result<LibraryState, String>),
    RestoredLibrary(Result<(LibraryState, CatalogPage, ServerClient), String>),
    Catalog(Result<(ScanSummary, CatalogPage, ServerClient), String>),
    CatalogPage(Result<CatalogPage, String>),
    Playback {
        state: Result<QueueSnapshot, String>,
        command_error: Option<String>,
    },
}

enum PlaybackRequest {
    Append(Vec<PathBuf>),
    AppendAllCatalog,
    PlayQueueEntry(u64),
    Previous,
    Skip,
    Reorder(u64, usize),
    Remove(u64),
    Clear,
    Repeat(RepeatMode),
    Shuffle(bool),
    Toggle(Vec<PathBuf>),
    Seek(u64),
    Stop,
    State,
}

struct LibraryUi {
    roots_list: gtk::ListBox,
    catalog_list: gtk::ListBox,
    roots_model: Rc<RefCell<Vec<PathBuf>>>,
    catalog_entries: Rc<RefCell<Vec<LibraryEntry>>>,
    catalog_paging: Rc<RefCell<Option<(ServerClient, usize)>>>,
    status: gtk::Label,
    scan_button: gtk::Button,
    load_more: gtk::Button,
    queue: QueueUi,
    playback: PlaybackUi,
    poll_playback: Arc<AtomicBool>,
}

#[derive(Clone)]
struct QueueUi {
    list: gtk::ListBox,
    entries: Rc<RefCell<Vec<QueueEntry>>>,
    current_entry_id: Rc<Cell<Option<u64>>>,
    syncing_controls: Rc<Cell<bool>>,
    server_available: Rc<Cell<bool>>,
    repeat: gtk::DropDown,
    shuffle: gtk::CheckButton,
    previous: gtk::Button,
    toggle: gtk::Button,
    play: gtk::Button,
    skip: gtk::Button,
    move_up: gtk::Button,
    move_down: gtk::Button,
    remove: gtk::Button,
    clear: gtk::Button,
}

struct PlaybackUi {
    current_track: gtk::Label,
    position: gtk::Scale,
    toggle: gtk::Button,
    stop: gtk::Button,
}

fn install_library_events(receiver: Receiver<UiEvent>, ui: LibraryUi) {
    let LibraryUi {
        roots_list,
        catalog_list,
        roots_model,
        catalog_entries,
        catalog_paging,
        status,
        scan_button,
        load_more,
        queue,
        playback,
        poll_playback,
    } = ui;
    glib::timeout_add_local(Duration::from_millis(40), move || {
        while let Ok(event) = receiver.try_recv() {
            match event {
                UiEvent::RestoredLibrary(Ok((state, page, client))) => {
                    *roots_model.borrow_mut() = state.selected_roots.clone();
                    replace_root_rows(&roots_list, &state.selected_roots);
                    replace_catalog_rows(&catalog_list, &page.entries, &catalog_entries);
                    *catalog_paging.borrow_mut() = page.next_offset.map(|offset| (client, offset));
                    load_more.set_visible(page.next_offset.is_some());
                    load_more.set_sensitive(page.next_offset.is_some());
                    status.set_text(&format!(
                        "Restored {} cached tracks. Scan Library to refresh.",
                        state.catalog_entries
                    ));
                }
                UiEvent::RestoredLibrary(Err(error)) => {
                    status.set_text(&format!("Could not restore library: {error}"));
                }
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
                    replace_catalog_rows(&catalog_list, &page.entries, &catalog_entries);
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
                    append_catalog_rows(&catalog_list, &page.entries, &catalog_entries);
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
                UiEvent::Playback {
                    state,
                    command_error,
                } => {
                    match state {
                        Ok(snapshot) => {
                            playback.apply(&snapshot.playback, &catalog_entries.borrow(), &status);
                            queue.apply(&snapshot, &catalog_entries.borrow());
                            poll_playback.store(
                                matches!(
                                    snapshot.playback.status,
                                    PlaybackStatus::Playing | PlaybackStatus::Paused
                                ),
                                Ordering::Relaxed,
                            );
                        }
                        Err(error) => {
                            poll_playback.store(false, Ordering::Relaxed);
                            status.set_text(&format!("Playback failed: {error}"));
                            playback.current_track.set_text("Playback error");
                            playback.toggle.set_sensitive(false);
                            playback.stop.set_sensitive(false);
                            queue.set_available(false);
                        }
                    }
                    if let Some(error) = command_error {
                        status.set_text(&error);
                    }
                }
            }
        }
        glib::ControlFlow::Continue
    });
}

fn send_playback_operation(
    client: ServerClient,
    request: PlaybackRequest,
    sender: Sender<UiEvent>,
) {
    thread::spawn(move || {
        let (state, command_error) = match perform_playback_request(&client, request) {
            Ok(snapshot) => (Ok(snapshot), None),
            Err(error) => match client.queue_state() {
                Ok(snapshot) => (Ok(snapshot), Some(error.to_string())),
                Err(state_error) => (
                    Err(format!(
                        "{error}; could not refresh playback state: {state_error}"
                    )),
                    None,
                ),
            },
        };
        let _ = sender.send(UiEvent::Playback {
            state,
            command_error,
        });
    });
}

fn perform_playback_request(
    client: &ServerClient,
    request: PlaybackRequest,
) -> std::io::Result<QueueSnapshot> {
    match request {
        PlaybackRequest::Append(tracks) => client.append_to_queue(&tracks),
        PlaybackRequest::AppendAllCatalog => {
            let mut offset = 0;
            let mut tracks = Vec::new();
            loop {
                let page = client.catalog_page(offset, 32)?;
                tracks.extend(
                    page.entries
                        .into_iter()
                        .filter(|entry| entry.active)
                        .map(|entry| entry.path),
                );
                let Some(next_offset) = page.next_offset else {
                    break;
                };
                offset = next_offset;
            }
            client.append_to_queue(&tracks)
        }
        PlaybackRequest::PlayQueueEntry(entry_id) => client.play_queue_entry(entry_id),
        PlaybackRequest::Previous => client.previous_queue_entry(),
        PlaybackRequest::Skip => client.skip_queue_entry(),
        PlaybackRequest::Reorder(entry_id, position) => {
            client.reorder_queue_entry(entry_id, position)
        }
        PlaybackRequest::Remove(entry_id) => client.remove_queue_entry(entry_id),
        PlaybackRequest::Clear => client.clear_queue(),
        PlaybackRequest::Repeat(mode) => client.set_repeat_mode(mode),
        PlaybackRequest::Shuffle(enabled) => client.set_shuffle(enabled),
        PlaybackRequest::Toggle(tracks) => {
            let state = client.queue_state()?;
            match state.playback.status {
                PlaybackStatus::Playing => {
                    client.pause_playback()?;
                    client.queue_state()
                }
                PlaybackStatus::Paused => {
                    client.resume_playback()?;
                    client.queue_state()
                }
                _ if !tracks.is_empty() => {
                    let queue = client.append_to_queue(&tracks)?;
                    let first_new_entry = queue
                        .entries
                        .get(queue.entries.len().saturating_sub(tracks.len()));
                    match first_new_entry {
                        Some(entry) => client.play_queue_entry(entry.id),
                        None => Ok(queue),
                    }
                }
                _ => match state
                    .current_entry_id
                    .or_else(|| state.entries.first().map(|e| e.id))
                {
                    Some(entry_id) => client.play_queue_entry(entry_id),
                    None => Ok(state),
                },
            }
        }
        PlaybackRequest::Seek(position_ms) => {
            client.seek_playback(position_ms)?;
            client.queue_state()
        }
        PlaybackRequest::Stop => {
            client.stop_playback()?;
            client.queue_state()
        }
        PlaybackRequest::State => client.queue_state(),
    }
}

fn install_playback_poll(
    server_state: Rc<RefCell<ServerState>>,
    sender: Sender<UiEvent>,
    enabled: Arc<AtomicBool>,
) {
    let busy = Arc::new(AtomicBool::new(false));
    glib::timeout_add_local(Duration::from_millis(250), move || {
        if !enabled.load(Ordering::Relaxed) || busy.swap(true, Ordering::Relaxed) {
            return glib::ControlFlow::Continue;
        }
        let Some(client) = server_state.borrow().client() else {
            busy.store(false, Ordering::Relaxed);
            let _ = sender.send(UiEvent::Playback {
                state: Err("Playback server is unavailable. Select Retry.".to_owned()),
                command_error: None,
            });
            return glib::ControlFlow::Continue;
        };
        let sender = sender.clone();
        let busy = Arc::clone(&busy);
        thread::spawn(move || {
            let result = perform_playback_request(&client, PlaybackRequest::State)
                .map_err(|error| error.to_string());
            let _ = sender.send(UiEvent::Playback {
                state: result,
                command_error: None,
            });
            busy.store(false, Ordering::Relaxed);
        });
        glib::ControlFlow::Continue
    });
}

impl PlaybackUi {
    fn apply(&self, snapshot: &PlaybackSnapshot, entries: &[LibraryEntry], status: &gtk::Label) {
        let track_detail = snapshot.track.as_ref().map_or_else(
            || "".to_owned(),
            |path| {
                entries
                    .iter()
                    .find(|entry| &entry.path == path)
                    .map(|entry| {
                        let view = present_catalog_entry(entry);
                        let title = view.title.as_deref().unwrap_or("(No title tag)");
                        let artists = if view.artists.is_empty() {
                            "(No artist tag)".to_owned()
                        } else {
                            view.artists.join(", ")
                        };
                        format!("{artists} — {title}")
                    })
                    .unwrap_or_else(|| path.display().to_string())
            },
        );
        let state_text = match snapshot.status {
            PlaybackStatus::Stopped => "Stopped".to_owned(),
            PlaybackStatus::Playing => "Playing".to_owned(),
            PlaybackStatus::Paused => "Paused".to_owned(),
            PlaybackStatus::Ended => "Finished".to_owned(),
            PlaybackStatus::Failed => "Playback error".to_owned(),
        };
        if snapshot.status == PlaybackStatus::Failed {
            status.set_text(
                snapshot
                    .error
                    .as_deref()
                    .unwrap_or("GStreamer could not play this audio file."),
            );
        } else {
            status.set_text(&state_text);
        }
        if snapshot.track.is_some() {
            let elapsed = format_time(snapshot.position_ms);
            let duration = snapshot
                .duration_ms
                .map(format_time)
                .unwrap_or_else(|| "Duration unavailable".to_owned());
            self.current_track.set_text(&format!(
                "{state_text}: {track_detail} · {elapsed} / {duration}"
            ));
        } else {
            self.current_track.set_text("Nothing is playing.");
        }
        let duration_ms = snapshot.duration_ms.unwrap_or(1).max(1);
        self.position.set_range(0.0, duration_ms as f64);
        self.position
            .set_value(snapshot.position_ms.min(duration_ms) as f64);
        let active = matches!(
            snapshot.status,
            PlaybackStatus::Playing | PlaybackStatus::Paused
        );
        self.stop
            .set_sensitive(active || snapshot.status == PlaybackStatus::Ended);
        self.position
            .set_sensitive(active && snapshot.duration_ms.is_some());
    }
}

impl QueueUi {
    fn apply(&self, snapshot: &QueueSnapshot, catalog_entries: &[LibraryEntry]) {
        let entries_changed = self.entries.borrow().as_slice() != snapshot.entries.as_slice();
        let current_changed = self.current_entry_id.get() != snapshot.current_entry_id;
        if entries_changed || current_changed {
            let selected_entry_id = self
                .list
                .selected_row()
                .and_then(|row| self.entries.borrow().get(row.index() as usize).cloned())
                .map(|entry| entry.id);
            replace_queue_rows(
                &self.list,
                &snapshot.entries,
                snapshot.current_entry_id,
                catalog_entries,
            );
            *self.entries.borrow_mut() = snapshot.entries.clone();
            self.current_entry_id.set(snapshot.current_entry_id);
            let row_to_select =
                selected_entry_id
                    .or(snapshot.current_entry_id)
                    .and_then(|entry_id| {
                        snapshot
                            .entries
                            .iter()
                            .position(|entry| entry.id == entry_id)
                    });
            if let Some(position) = row_to_select {
                self.list
                    .select_row(self.list.row_at_index(position as i32).as_ref());
            }
        }

        self.syncing_controls.set(true);
        self.repeat.set_selected(match snapshot.repeat_mode {
            RepeatMode::Off => 0,
            RepeatMode::One => 1,
            RepeatMode::All => 2,
        });
        self.shuffle.set_active(snapshot.shuffle);
        self.syncing_controls.set(false);

        self.update_actions();
    }

    fn set_available(&self, available: bool) {
        self.server_available.set(available);
        self.repeat.set_sensitive(available);
        self.shuffle.set_sensitive(available);
        self.toggle.set_sensitive(available);
        self.update_actions();
    }

    fn update_actions(&self) {
        if !self.server_available.get() {
            self.previous.set_sensitive(false);
            self.play.set_sensitive(false);
            self.skip.set_sensitive(false);
            self.move_up.set_sensitive(false);
            self.move_down.set_sensitive(false);
            self.remove.set_sensitive(false);
            self.clear.set_sensitive(false);
            return;
        }
        let selected_position = self.list.selected_row().map(|row| row.index() as usize);
        let entries = self.entries.borrow();
        let current_position = self
            .current_entry_id
            .get()
            .and_then(|current_id| entries.iter().position(|entry| entry.id == current_id));
        let selected_is_upcoming = selected_position
            .is_some_and(|position| current_position.is_none_or(|current| position > current));
        self.play.set_sensitive(selected_position.is_some());
        self.previous
            .set_sensitive(self.current_entry_id.get().is_some());
        self.skip
            .set_sensitive(self.current_entry_id.get().is_some());
        self.move_up.set_sensitive(
            selected_is_upcoming
                && selected_position.is_some_and(|position| {
                    position > current_position.map_or(0, |current| current + 1)
                }),
        );
        self.move_down.set_sensitive(
            selected_is_upcoming
                && selected_position.is_some_and(|position| position + 1 < entries.len()),
        );
        self.remove.set_sensitive(selected_position.is_some());
        self.clear.set_sensitive(!entries.is_empty());
    }
}

fn selected_active_tracks(
    list: &gtk::ListBox,
    catalog_entries: &Rc<RefCell<Vec<LibraryEntry>>>,
) -> Vec<PathBuf> {
    let mut rows = list.selected_rows();
    rows.sort_by_key(|row| row.index());
    rows.into_iter()
        .filter_map(|row| {
            catalog_entries
                .borrow()
                .get(row.index() as usize)
                .filter(|entry| entry.active)
                .map(|entry| entry.path.clone())
        })
        .collect()
}

fn replace_queue_rows(
    list: &gtk::ListBox,
    entries: &[QueueEntry],
    current_entry_id: Option<u64>,
    catalog_entries: &[LibraryEntry],
) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }
    for entry in entries {
        let title = catalog_entries
            .iter()
            .find(|catalog_entry| catalog_entry.path == entry.track)
            .map(|catalog_entry| {
                let view = present_catalog_entry(catalog_entry);
                let title = view.title.as_deref().unwrap_or("(No title tag)");
                if view.artists.is_empty() {
                    title.to_owned()
                } else {
                    format!("{} — {title}", view.artists.join(", "))
                }
            })
            .unwrap_or_else(|| entry.track.display().to_string());
        let marker = match (current_entry_id == Some(entry.id), entry.failed) {
            (true, true) => "▶ Failed · ",
            (true, false) => "▶ ",
            (false, true) => "Failed · ",
            (false, false) => "",
        };
        let label = gtk::Label::new(Some(&format!("{marker}{title}")));
        label.set_halign(gtk::Align::Start);
        label.set_wrap(true);
        label.set_selectable(true);
        if entry.failed {
            label.add_css_class("error");
        }
        let row = gtk::ListBoxRow::new();
        row.set_child(Some(&label));
        list.append(&row);
    }
}

fn format_time(milliseconds: u64) -> String {
    let seconds = milliseconds / 1_000;
    format!("{}:{:02}", seconds / 60, seconds % 60)
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

fn replace_catalog_rows(
    list: &gtk::ListBox,
    entries: &[LibraryEntry],
    catalog_entries: &Rc<RefCell<Vec<LibraryEntry>>>,
) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }
    catalog_entries.borrow_mut().clear();
    append_catalog_rows(list, entries, catalog_entries);
}

fn append_catalog_rows(
    list: &gtk::ListBox,
    entries: &[LibraryEntry],
    catalog_entries: &Rc<RefCell<Vec<LibraryEntry>>>,
) {
    for entry in entries {
        list.append(&render_catalog_row(entry));
    }
    catalog_entries.borrow_mut().extend_from_slice(entries);
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

fn read_restored_library(
    client: &ServerClient,
) -> Result<(LibraryState, CatalogPage, ServerClient), String> {
    let state = client.library_state().map_err(|error| error.to_string())?;
    let page = client
        .catalog_page(0, 32)
        .map_err(|error| error.to_string())?;
    Ok((state, page, client.clone()))
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

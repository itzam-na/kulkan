mod catalog_selection;

use std::cell::{Cell, RefCell};
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Duration;

use catalog_selection::{SelectionModifiers, SelectionState};
use gtk::gio;
use gtk::glib;
use gtk::prelude::*;
use kulkan_library::{
    CatalogPage, LibraryEntry, LibraryState, NumericFilter, ScanSummary, SearchCriteria,
    TextFilter, present_catalog_entry,
};
use kulkan_runtime::{
    ClientInstance, PlayCountUpdate, PlaybackSnapshot, PlaybackStatus, QueueEntry, QueueSnapshot,
    RepeatMode, ServerClient, ServerProcess, activate_existing_client,
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

    let content = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    content.set_vexpand(true);
    content.set_margin_top(16);
    content.set_margin_bottom(16);
    content.set_margin_start(16);
    content.set_margin_end(16);

    let library_sidebar = gtk::Box::new(gtk::Orientation::Vertical, 8);
    library_sidebar.set_size_request(216, -1);
    library_sidebar.add_css_class("sidebar");
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
    roots_list.update_property(&[gtk::accessible::Property::Label("Library folders")]);
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
    let library_settings = gtk::Expander::new(Some("Library settings & data"));
    let library_settings_content = gtk::Box::new(gtk::Orientation::Vertical, 8);
    library_settings_content.set_margin_top(8);
    library_settings_content.set_margin_start(8);
    library_settings_content.set_margin_end(8);
    let threshold_label = gtk::Label::new(Some("Played threshold (%)"));
    threshold_label.set_halign(gtk::Align::Start);
    library_settings_content.append(&threshold_label);
    let played_threshold = gtk::SpinButton::with_range(1.0, 100.0, 1.0);
    played_threshold.set_value(75.0);
    played_threshold.set_sensitive(false);
    played_threshold.set_tooltip_text(Some("The share of a track needed to count as played."));
    played_threshold
        .update_property(&[gtk::accessible::Property::Label("Played threshold percent")]);
    library_settings_content.append(&played_threshold);
    let clear_all_history = gtk::Button::with_label("Clear All Play History");
    clear_all_history.set_sensitive(false);
    library_settings_content.append(&clear_all_history);
    let export_state = gtk::Button::with_label("Export State…");
    let import_state = gtk::Button::with_label("Import State…");
    let reset_state = gtk::Button::with_label("Reset App State…");
    export_state.set_sensitive(false);
    import_state.set_sensitive(false);
    reset_state.set_sensitive(false);
    library_settings_content.append(&export_state);
    library_settings_content.append(&import_state);
    library_settings_content.append(&reset_state);
    library_settings.set_child(Some(&library_settings_content));
    library_sidebar.append(&library_settings);

    let main_content = gtk::Box::new(gtk::Orientation::Vertical, 8);
    let status = gtk::Label::new(None);
    status.set_wrap(true);
    status.set_selectable(true);
    status.set_halign(gtk::Align::Start);
    main_content.append(&status);

    let basic_search_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let basic_search = gtk::Entry::new();
    basic_search.set_hexpand(true);
    basic_search.set_placeholder_text(Some("Search title, artist, album, genre, or location"));
    basic_search.update_property(&[gtk::accessible::Property::Label("Search the Library")]);
    let not_yet_played_filter =
        gtk::DropDown::from_strings(&["Any play status", "Not-yet-played", "not Not-yet-played"]);
    not_yet_played_filter
        .update_property(&[gtk::accessible::Property::Label("Not-yet-played filter")]);
    let search_button = gtk::Button::with_label("Search");
    let clear_search = gtk::Button::with_label("Clear");
    search_button.set_sensitive(false);
    basic_search_row.append(&basic_search);
    basic_search_row.append(&not_yet_played_filter);
    basic_search_row.append(&search_button);
    basic_search_row.append(&clear_search);
    main_content.append(&basic_search_row);
    let search_chips = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    search_chips.set_visible(false);
    main_content.append(&search_chips);

    let advanced_search = gtk::Expander::new(Some("Advanced search"));
    let advanced_grid = gtk::Grid::new();
    advanced_grid.set_column_spacing(8);
    advanced_grid.set_row_spacing(6);
    advanced_grid.set_margin_start(8);
    advanced_grid.set_margin_end(8);
    let artist_filter = gtk::Entry::new();
    artist_filter.set_placeholder_text(Some("Artist contains"));
    artist_filter.update_property(&[gtk::accessible::Property::Label("Artist")]);
    let artist_missing = gtk::CheckButton::with_label("is missing");
    artist_missing.update_property(&[gtk::accessible::Property::Label("Artist is missing")]);
    let album_filter = gtk::Entry::new();
    album_filter.set_placeholder_text(Some("Album contains"));
    album_filter.update_property(&[gtk::accessible::Property::Label("Album")]);
    let album_missing = gtk::CheckButton::with_label("is missing");
    album_missing.update_property(&[gtk::accessible::Property::Label("Album is missing")]);
    let genre_filter = gtk::Entry::new();
    genre_filter.set_placeholder_text(Some("Genre contains"));
    genre_filter.update_property(&[gtk::accessible::Property::Label("Genre")]);
    let genre_missing = gtk::CheckButton::with_label("is missing");
    genre_missing.update_property(&[gtk::accessible::Property::Label("Genre is missing")]);
    let year_filter = gtk::Entry::new();
    year_filter.set_placeholder_text(Some("exact or min..max"));
    year_filter.update_property(&[gtk::accessible::Property::Label("Release year")]);
    let year_missing = gtk::CheckButton::with_label("is missing");
    year_missing.update_property(&[gtk::accessible::Property::Label("Release year is missing")]);
    let duration_filter = gtk::Entry::new();
    duration_filter.set_placeholder_text(Some("seconds: exact or min..max"));
    duration_filter.update_property(&[gtk::accessible::Property::Label("Duration in seconds")]);
    let location_filter = gtk::Entry::new();
    location_filter.set_placeholder_text(Some("File location contains"));
    location_filter.update_property(&[gtk::accessible::Property::Label("File location")]);
    let play_count_filter = gtk::Entry::new();
    play_count_filter.set_placeholder_text(Some("exact or min..max"));
    play_count_filter.update_property(&[gtk::accessible::Property::Label("Play count")]);
    for (row, label, entry, missing) in [
        (0, "Artist", &artist_filter, Some(&artist_missing)),
        (1, "Album", &album_filter, Some(&album_missing)),
        (2, "Genre", &genre_filter, Some(&genre_missing)),
        (3, "Release year", &year_filter, Some(&year_missing)),
        (4, "Duration", &duration_filter, None),
        (5, "File location", &location_filter, None),
        (6, "Play count", &play_count_filter, None),
    ] {
        let label = gtk::Label::new(Some(label));
        label.set_halign(gtk::Align::Start);
        entry.set_hexpand(true);
        advanced_grid.attach(&label, 0, row, 1, 1);
        advanced_grid.attach(entry, 1, row, 1, 1);
        if let Some(missing) = missing {
            advanced_grid.attach(missing, 2, row, 1, 1);
        }
    }
    advanced_search.set_child(Some(&advanced_grid));
    main_content.append(&advanced_search);

    let idle_panel = gtk::Box::new(gtk::Orientation::Vertical, 12);
    idle_panel.set_vexpand(true);
    idle_panel.set_valign(gtk::Align::Center);
    idle_panel.set_halign(gtk::Align::Center);
    idle_panel.add_css_class("card");
    idle_panel.set_margin_start(24);
    idle_panel.set_margin_end(24);
    idle_panel.set_margin_top(24);
    idle_panel.set_margin_bottom(24);
    let empty_library_title = gtk::Label::new(Some("Your Library is empty"));
    empty_library_title.add_css_class("title-1");
    idle_panel.append(&empty_library_title);
    let empty_library_message = gtk::Label::new(Some(
        "Select Add Folder…, then choose Scan Library to add your tracks here.",
    ));
    empty_library_message.add_css_class("dim-label");
    empty_library_message.set_wrap(true);
    empty_library_message.set_justify(gtk::Justification::Center);
    idle_panel.append(&empty_library_message);
    main_content.append(&idle_panel);

    let results_panel = gtk::Box::new(gtk::Orientation::Vertical, 8);
    results_panel.set_vexpand(true);
    results_panel.set_visible(false);
    let catalog_label = gtk::Label::new(Some("Library"));
    catalog_label.set_halign(gtk::Align::Start);
    catalog_label.add_css_class("title-2");
    results_panel.append(&catalog_label);
    let catalog_header = gtk::Grid::new();
    catalog_header.set_hexpand(true);
    catalog_header.set_column_spacing(12);
    catalog_header.set_margin_start(8);
    catalog_header.set_margin_end(8);
    for (column, title) in ["Track", "Album", "Duration", "Plays", ""]
        .into_iter()
        .enumerate()
    {
        let label = gtk::Label::new(Some(title));
        label.set_halign(gtk::Align::Start);
        label.add_css_class("heading");
        label.add_css_class("dim-label");
        match column {
            0 => {
                label.set_width_chars(18);
                label.set_hexpand(true);
            }
            1 => label.set_width_chars(14),
            2 => label.set_width_chars(8),
            3 => label.set_width_chars(5),
            _ => label.set_width_chars(12),
        }
        catalog_header.attach(&label, column as i32, 0, 1, 1);
    }
    results_panel.append(&catalog_header);
    let no_matches = gtk::Label::new(Some("No tracks match this search."));
    no_matches.set_visible(false);
    no_matches.set_margin_top(24);
    no_matches.add_css_class("dim-label");
    results_panel.append(&no_matches);
    let catalog_list = gtk::ListBox::new();
    catalog_list.set_selection_mode(gtk::SelectionMode::Single);
    catalog_list.update_property(&[gtk::accessible::Property::Label("Library tracks")]);
    catalog_list.add_css_class("boxed-list");
    let catalog_scroll = gtk::ScrolledWindow::builder()
        .child(&catalog_list)
        .vexpand(true)
        .hexpand(true)
        .build();
    results_panel.append(&catalog_scroll);
    let load_more = gtk::Button::with_label("Load More Tracks");
    load_more.set_visible(false);
    results_panel.append(&load_more);
    main_content.append(&results_panel);

    let current_track = gtk::Label::new(Some("Nothing is playing"));
    current_track.set_halign(gtk::Align::Start);
    current_track.set_ellipsize(gtk::pango::EllipsizeMode::End);
    current_track.set_hexpand(true);
    current_track.add_css_class("heading");
    let current_track_subtitle = gtk::Label::new(Some(""));
    current_track_subtitle.set_halign(gtk::Align::Start);
    current_track_subtitle.set_ellipsize(gtk::pango::EllipsizeMode::End);
    current_track_subtitle.add_css_class("dim-label");
    let current_track_details = gtk::Box::new(gtk::Orientation::Vertical, 2);
    current_track_details.set_hexpand(true);
    current_track_details.append(&current_track);
    current_track_details.append(&current_track_subtitle);
    let playback_controls = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    playback_controls.add_css_class("linked");
    let previous_track = gtk::Button::new();
    previous_track.set_child(Some(&gtk::Image::from_icon_name(
        "media-skip-backward-symbolic",
    )));
    previous_track.set_tooltip_text(Some("Previous track"));
    previous_track.update_property(&[gtk::accessible::Property::Label("Previous track")]);
    let stop_playback = gtk::Button::new();
    stop_playback.set_child(Some(&gtk::Image::from_icon_name(
        "media-playback-stop-symbolic",
    )));
    stop_playback.set_tooltip_text(Some("Stop playback"));
    stop_playback.update_property(&[gtk::accessible::Property::Label("Stop playback")]);
    let play_pause = gtk::Button::new();
    let play_pause_icon = gtk::Image::from_icon_name("media-playback-start-symbolic");
    play_pause.set_child(Some(&play_pause_icon));
    play_pause.set_tooltip_text(Some("Play or pause"));
    play_pause.update_property(&[gtk::accessible::Property::Label("Play or pause")]);
    let next_track = gtk::Button::new();
    next_track.set_child(Some(&gtk::Image::from_icon_name(
        "media-skip-forward-symbolic",
    )));
    next_track.set_tooltip_text(Some("Next track"));
    next_track.update_property(&[gtk::accessible::Property::Label("Next track")]);
    previous_track.set_sensitive(false);
    stop_playback.set_sensitive(false);
    play_pause.set_sensitive(false);
    next_track.set_sensitive(false);
    for button in [&previous_track, &stop_playback, &play_pause, &next_track] {
        button.add_css_class("flat");
        playback_controls.append(button);
    }
    playback_controls.set_hexpand(true);
    playback_controls.set_halign(gtk::Align::Center);
    let playback_top = gtk::Grid::new();
    playback_top.set_column_homogeneous(true);
    playback_top.attach(&current_track_details, 0, 0, 1, 1);
    playback_top.attach(&playback_controls, 1, 0, 1, 1);
    playback_top.attach(&gtk::Box::new(gtk::Orientation::Horizontal, 0), 2, 0, 1, 1);
    let playback_position = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 1.0, 1.0);
    playback_position.set_hexpand(true);
    playback_position.set_draw_value(false);
    playback_position.set_sensitive(false);
    playback_position.set_tooltip_text(Some("Click the timeline to seek."));
    let playback_elapsed = gtk::Label::new(Some("0:00"));
    playback_elapsed.add_css_class("numeric");
    let playback_duration = gtk::Label::new(Some("0:00"));
    playback_duration.add_css_class("numeric");
    let playback_timeline = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    playback_timeline.append(&playback_elapsed);
    playback_timeline.append(&playback_position);
    playback_timeline.append(&playback_duration);
    let playback_bar = gtk::Box::new(gtk::Orientation::Vertical, 6);
    playback_bar.add_css_class("toolbar");
    playback_bar.set_margin_start(16);
    playback_bar.set_margin_end(16);
    playback_bar.set_margin_bottom(8);
    playback_bar.append(&playback_top);
    playback_bar.append(&playback_timeline);

    let queue_placeholder = gtk::Box::new(gtk::Orientation::Vertical, 8);
    queue_placeholder.set_size_request(264, -1);
    queue_placeholder.add_css_class("sidebar");
    let queue_title = gtk::Label::new(Some("Queue"));
    queue_title.add_css_class("title-2");
    queue_title.set_halign(gtk::Align::Start);
    queue_placeholder.append(&queue_title);
    let queue_list = gtk::ListBox::new();
    queue_list.set_selection_mode(gtk::SelectionMode::Single);
    queue_list.update_property(&[gtk::accessible::Property::Label("Playback Queue")]);
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
    shell.append(&playback_bar);
    window.set_child(Some(&shell));

    let roots_model = Rc::new(RefCell::new(Vec::<PathBuf>::new()));
    let catalog_entries = Rc::new(RefCell::new(Vec::<LibraryEntry>::new()));
    let catalog_details = Rc::new(RefCell::new(Vec::<LibraryEntry>::new()));
    let queue_entries = Rc::new(RefCell::new(Vec::<QueueEntry>::new()));
    let displayed_queue_current = Rc::new(Cell::new(None));
    let syncing_queue_controls = Rc::new(Cell::new(false));
    let poll_playback = Arc::new(AtomicBool::new(false));
    let recovery_attempted = Rc::new(Cell::new(false));
    let (sender, receiver) = mpsc::channel();
    let catalog_paging = Rc::new(RefCell::new(None::<(ServerClient, SearchCriteria, usize)>));
    let active_search = Rc::new(RefCell::new(None::<SearchCriteria>));
    let search_generation = Rc::new(Cell::new(0_u64));
    let catalog_actions = CatalogActions {
        server_state: Rc::clone(&server_state),
        sender: sender.clone(),
        status: status.clone(),
        active_search: Rc::clone(&active_search),
    };
    let search_form = SearchForm {
        text: basic_search.clone(),
        not_yet_played: not_yet_played_filter.clone(),
        artist: artist_filter.clone(),
        artist_missing: artist_missing.clone(),
        album: album_filter.clone(),
        album_missing: album_missing.clone(),
        genre: genre_filter.clone(),
        genre_missing: genre_missing.clone(),
        year: year_filter.clone(),
        year_missing: year_missing.clone(),
        duration: duration_filter.clone(),
        location: location_filter.clone(),
        play_count: play_count_filter.clone(),
    };
    let syncing_threshold = Rc::new(Cell::new(false));
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
            catalog_label: catalog_label.clone(),
            clear_search: clear_search.clone(),
            add_root: add_root.clone(),
            remove_root: remove_root.clone(),
            search_button: search_button.clone(),
            add_selected: add_selected.clone(),
            add_all: add_all.clone(),
            export_state: export_state.clone(),
            import_state: import_state.clone(),
            reset_state: reset_state.clone(),
            retry: retry.clone(),
            roots_model: Rc::clone(&roots_model),
            catalog_entries: Rc::clone(&catalog_entries),
            catalog_details: Rc::clone(&catalog_details),
            catalog_paging: Rc::clone(&catalog_paging),
            active_search: Rc::clone(&active_search),
            search_generation: Rc::clone(&search_generation),
            actions: catalog_actions.clone(),
            search_chips: search_chips.clone(),
            idle_panel: idle_panel.clone(),
            results_panel: results_panel.clone(),
            no_matches: no_matches.clone(),
            status: status.clone(),
            scan_button: scan_button.clone(),
            load_more: load_more.clone(),
            played_threshold: played_threshold.clone(),
            syncing_threshold: Rc::clone(&syncing_threshold),
            clear_all_history: clear_all_history.clone(),
            recovery_attempted: Rc::clone(&recovery_attempted),
            queue: queue_ui.clone(),
            playback: PlaybackUi {
                current_track: current_track.clone(),
                current_track_subtitle: current_track_subtitle.clone(),
                position: playback_position.clone(),
                elapsed: playback_elapsed.clone(),
                duration: playback_duration.clone(),
                toggle: play_pause.clone(),
                toggle_icon: play_pause_icon.clone(),
                stop: stop_playback.clone(),
            },
            poll_playback: Arc::clone(&poll_playback),
        },
    );
    let selection_queue_ui = queue_ui.clone();
    queue_list.connect_row_selected(move |_, _| {
        selection_queue_ui.update_actions();
    });

    let search_state = Rc::clone(&server_state);
    let search_sender = sender.clone();
    let search_request_generation = Rc::clone(&search_generation);
    let search_criteria_form = search_form.clone();
    let search_status = status.clone();
    let search_idle_panel = idle_panel.clone();
    let search_results_panel = results_panel.clone();
    let search_add_all = add_all.clone();
    let search_label = catalog_label.clone();
    let search_no_matches = no_matches.clone();
    search_button.connect_clicked(move |_| {
        let criteria = match search_criteria_form.criteria() {
            Ok(criteria) => criteria,
            Err(error) => {
                search_status.set_text(&error);
                return;
            }
        };
        let Some(client) = search_state.borrow().client() else {
            search_status.set_text("Server unavailable. Select Retry.");
            return;
        };
        let generation = search_request_generation.get().wrapping_add(1);
        search_request_generation.set(generation);
        search_idle_panel.set_visible(false);
        search_results_panel.set_visible(true);
        search_no_matches.set_visible(false);
        search_add_all.set_label("Add All Matches");
        search_label.set_text("Matching Tracks");
        search_status.set_text("Searching the Library…");
        send_search(client, criteria, generation, search_sender.clone());
    });
    let search_button_for_enter = search_button.clone();
    basic_search.connect_activate(move |_| search_button_for_enter.emit_clicked());

    let clear_search_form = search_form.clone();
    let clear_search_model = Rc::clone(&active_search);
    let clear_search_generation = Rc::clone(&search_generation);
    let clear_search_paging = Rc::clone(&catalog_paging);
    let clear_search_add_all = add_all.clone();
    let clear_search_status = status.clone();
    let clear_search_label = catalog_label.clone();
    let clear_search_list = catalog_list.clone();
    let clear_search_chips = search_chips.clone();
    let clear_search_no_matches = no_matches.clone();
    let clear_search_state = Rc::clone(&server_state);
    let clear_search_sender = sender.clone();
    let clear_search_results_panel = results_panel.clone();
    let clear_search_empty_panel = idle_panel.clone();
    let clear_search_load_more = load_more.clone();
    let clear_search_library_entries = Rc::clone(&catalog_entries);
    let clear_search_library_actions = catalog_actions.clone();
    clear_search.connect_clicked(move |_| {
        clear_search_form.reset();
        let generation = clear_search_generation.get().wrapping_add(1);
        clear_search_generation.set(generation);
        clear_search_model.borrow_mut().take();
        clear_search_paging.borrow_mut().take();
        clear_search_load_more.set_visible(false);
        clear_search_load_more.set_sensitive(false);
        replace_catalog_rows(
            &clear_search_list,
            &[],
            &clear_search_library_entries,
            &clear_search_library_actions,
        );
        replace_search_chips(&clear_search_chips, &SearchCriteria::default());
        clear_search_no_matches.set_visible(false);
        clear_search_results_panel.set_visible(true);
        clear_search_empty_panel.set_visible(false);
        clear_search_add_all.set_label("Add All Tracks");
        clear_search_label.set_text("Library");
        clear_search_status.set_text("Loading Library…");
        if let Some(client) = clear_search_state.borrow().client() {
            send_search(
                client,
                SearchCriteria::default(),
                generation,
                clear_search_sender.clone(),
            );
        } else {
            clear_search_results_panel.set_visible(false);
            clear_search_empty_panel.set_visible(true);
            clear_search_status.set_text("Server unavailable. Select Retry.");
        }
    });
    let selection_model = Rc::clone(&catalog_entries);
    let add_for_selection = add_selected.clone();
    let selection_server = Rc::clone(&server_state);
    catalog_list.set_selection_mode(gtk::SelectionMode::Multiple);
    let selection_click = gtk::GestureClick::new();
    selection_click.set_button(1);
    selection_click.set_propagation_phase(gtk::PropagationPhase::Capture);
    let selection_list = catalog_list.clone();
    let selection_catalog_entries = Rc::clone(&catalog_entries);
    let selection_state = Rc::new(RefCell::new(SelectionState::default()));
    let selection_state_for_click = Rc::clone(&selection_state);
    selection_click.connect_pressed(move |gesture, _, x, y| {
        let mut picked_widget = selection_list.pick(x, y, gtk::PickFlags::DEFAULT);
        while let Some(widget) = picked_widget {
            if widget.is::<gtk::Button>() {
                return;
            }
            picked_widget = widget.parent();
        }
        let Some(row) = selection_list.row_at_y(y as i32) else {
            return;
        };
        if !row.is_selectable() {
            return;
        }
        let modifiers = gesture.current_event_state();
        let current_selection = selection_list
            .selected_rows()
            .iter()
            .map(|row| row.index() as usize)
            .collect::<BTreeSet<_>>();
        let selection = selection_state_for_click.borrow_mut().select(
            &current_selection,
            row.index() as usize,
            SelectionModifiers {
                control: modifiers.contains(gtk::gdk::ModifierType::CONTROL_MASK),
                shift: modifiers.contains(gtk::gdk::ModifierType::SHIFT_MASK),
            },
            selection_catalog_entries.borrow().len(),
        );
        selection_list.unselect_all();
        for index in selection {
            if let Some(row) = selection_list.row_at_index(index as i32)
                && row.is_selectable()
            {
                selection_list.select_row(Some(&row));
            }
        }
        row.grab_focus();
        gesture.set_state(gtk::EventSequenceState::Claimed);
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

    let threshold_syncing = Rc::clone(&syncing_threshold);
    let pending_threshold = Rc::new(Cell::new(None::<(u8, std::time::Instant)>));
    let pending_for_spin = Rc::clone(&pending_threshold);
    played_threshold.connect_value_changed(move |spin| {
        if threshold_syncing.get() {
            return;
        }
        pending_for_spin.set(Some((
            spin.value_as_int().clamp(1, 100) as u8,
            std::time::Instant::now(),
        )));
    });
    let threshold_state = Rc::clone(&server_state);
    let threshold_sender = sender.clone();
    glib::timeout_add_local(Duration::from_millis(100), move || {
        if let Some((percent, changed_at)) = pending_threshold.get()
            && changed_at.elapsed() >= Duration::from_millis(250)
        {
            pending_threshold.set(None);
            if let Some(client) = threshold_state.borrow().client() {
                send_threshold_operation(client, percent, threshold_sender.clone());
            }
        }
        glib::ControlFlow::Continue
    });

    let clear_all_state = Rc::clone(&server_state);
    let clear_all_sender = sender.clone();
    let clear_all_search = Rc::clone(&active_search);
    clear_all_history.connect_clicked(move |_| {
        if let Some(client) = clear_all_state.borrow().client() {
            send_clear_all_history(
                client,
                clear_all_search.borrow().clone(),
                clear_all_sender.clone(),
            );
        }
    });

    let export_parent = window.clone();
    let export_state_server = Rc::clone(&server_state);
    let export_status = status.clone();
    let export_sender = sender.clone();
    export_state.connect_clicked(move |_| {
        let chooser = gtk::FileChooserNative::new(
            Some("Export Kulkán State"),
            Some(&export_parent),
            gtk::FileChooserAction::Save,
            Some("Export"),
            Some("Cancel"),
        );
        chooser.set_current_name("kulkan-state.json");
        let state = Rc::clone(&export_state_server);
        let status = export_status.clone();
        let sender = export_sender.clone();
        chooser.connect_response(move |chooser, response| {
            if response == gtk::ResponseType::Accept {
                let Some(path) = chooser.file().and_then(|file| file.path()) else {
                    status.set_text("The export destination is not available as a local path.");
                    return;
                };
                let Some(client) = state.borrow().client() else {
                    status.set_text("Server unavailable. Select Retry.");
                    return;
                };
                send_export_state(client, path, sender.clone());
                status.set_text("Exporting app state…");
            }
            chooser.hide();
        });
        chooser.show();
    });

    let import_parent = window.clone();
    let import_state_server = Rc::clone(&server_state);
    let import_status = status.clone();
    let import_sender = sender.clone();
    import_state.connect_clicked(move |_| {
        let chooser = gtk::FileChooserNative::new(
            Some("Import Kulkán State"),
            Some(&import_parent),
            gtk::FileChooserAction::Open,
            Some("Import"),
            Some("Cancel"),
        );
        let state = Rc::clone(&import_state_server);
        let status = import_status.clone();
        let sender = import_sender.clone();
        chooser.connect_response(move |chooser, response| {
            if response == gtk::ResponseType::Accept {
                let Some(path) = chooser.file().and_then(|file| file.path()) else {
                    status.set_text("The selected backup is not available as a local path.");
                    return;
                };
                let Some(client) = state.borrow().client() else {
                    status.set_text("Server unavailable. Select Retry.");
                    return;
                };
                send_import_state(client, path, sender.clone());
                status.set_text("Importing app state…");
            }
            chooser.hide();
        });
        chooser.show();
    });

    let reset_parent = window.clone();
    let reset_server = Rc::clone(&server_state);
    let reset_sender = sender.clone();
    let reset_status = status.clone();
    reset_state.connect_clicked(move |_| {
        let dialog = gtk::MessageDialog::builder()
            .transient_for(&reset_parent)
            .modal(true)
            .message_type(gtk::MessageType::Warning)
            .text("Reset all Kulkán app state?")
            .secondary_text(
                "This clears selected folders, cached tracks, play history, preferences, and the Queue. Media files are not changed.",
            )
            .build();
        dialog.add_button("Cancel", gtk::ResponseType::Cancel);
        dialog.add_button("Reset", gtk::ResponseType::Accept);
        let state = Rc::clone(&reset_server);
        let sender = reset_sender.clone();
        let status = reset_status.clone();
        let dialog_lifetime = Rc::new(RefCell::new(Some(dialog.clone())));
        let dialog_lifetime_for_response = Rc::clone(&dialog_lifetime);
        dialog.connect_response(move |dialog, response| {
            if response == gtk::ResponseType::Accept {
                if let Some(client) = state.borrow().client() {
                    send_reset_app_state(client, sender.clone());
                    status.set_text("Resetting app state…");
                } else {
                    status.set_text("Server unavailable. Select Retry.");
                }
            }
            dialog.close();
            dialog_lifetime_for_response.borrow_mut().take();
        });
        dialog.present();
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
    let all_search = Rc::clone(&active_search);
    add_all.connect_clicked(move |_| {
        let Some(client) = all_state.borrow().client() else {
            all_status.set_text("Server unavailable. Select Retry.");
            return;
        };
        all_status.set_text("Adding all matching tracks to the Queue…");
        send_playback_operation(
            client,
            PlaybackRequest::AppendAllMatches(all_search.borrow().clone().unwrap_or_default()),
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
    let scan_search = Rc::clone(&active_search);
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
        let criteria = scan_search.borrow().clone();
        thread::spawn(move || {
            let _ = sender.send(UiEvent::Catalog(read_scanned_catalog(&client, criteria)));
        });
    });

    let paging_for_button = Rc::clone(&catalog_paging);
    let paging_sender = sender.clone();
    let load_more_status = status.clone();
    let load_more_button = load_more.clone();
    load_more.connect_clicked(move |_| {
        let Some((client, criteria, offset)) = paging_for_button.borrow().clone() else {
            return;
        };
        load_more_button.set_sensitive(false);
        load_more_status.set_text("Loading more tracks…");
        let sender = paging_sender.clone();
        thread::spawn(move || {
            let result = client
                .search_catalog(&criteria, offset, 32)
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
            search_button.set_sensitive(true);
            export_state.set_sensitive(true);
            import_state.set_sensitive(true);
            reset_state.set_sensitive(true);
            played_threshold.set_sensitive(true);
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
            search_button.set_sensitive(false);
            export_state.set_sensitive(false);
            import_state.set_sensitive(false);
            reset_state.set_sensitive(false);
            played_threshold.set_sensitive(false);
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
    let retry_threshold = played_threshold.clone();
    let retry_search = search_button.clone();
    let retry_export = export_state.clone();
    let retry_import = import_state.clone();
    let retry_reset = reset_state.clone();
    let retry_recovery_attempted = Rc::clone(&recovery_attempted);
    retry.connect_clicked(move |_| {
        let retry_result = { retry_state.borrow_mut().start() };
        match retry_result {
            Ok(()) => {
                retry_status.set_text("Server ready. Add a library folder, then scan it.");
                retry_button.set_sensitive(false);
                retry_recovery_attempted.set(false);
                retry_search.set_sensitive(true);
                retry_export.set_sensitive(true);
                retry_import.set_sensitive(true);
                retry_reset.set_sensitive(true);
                retry_threshold.set_sensitive(true);
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
                retry_search.set_sensitive(false);
                retry_export.set_sensitive(false);
                retry_import.set_sensitive(false);
                retry_reset.set_sensitive(false);
                retry_threshold.set_sensitive(false);
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

    let shortcut_controller = gtk::EventControllerKey::new();
    let shortcut_search = basic_search.clone();
    let shortcut_play_pause = play_pause.clone();
    let shortcut_previous = previous_track.clone();
    let shortcut_next = next_track.clone();
    let shortcut_scan = scan_button.clone();
    let shortcut_add_selected = add_selected.clone();
    let shortcut_export = export_state.clone();
    let shortcut_import = import_state.clone();
    let shortcut_clear = clear_search.clone();
    let shortcut_active_search = Rc::clone(&active_search);
    let shortcut_catalog_list = catalog_list.clone();
    let shortcut_queue_list = queue_list.clone();
    let shortcut_window = window.clone();
    shortcut_controller.set_propagation_phase(gtk::PropagationPhase::Capture);
    shortcut_controller.connect_key_pressed(move |_, key, _, modifiers| {
        let control = modifiers.contains(gtk::gdk::ModifierType::CONTROL_MASK);
        if control {
            if matches!(key, gtk::gdk::Key::f | gtk::gdk::Key::F) {
                shortcut_search.grab_focus();
                return glib::Propagation::Stop;
            }
            let text_input_focused = gtk::prelude::GtkWindowExt::focus(&shortcut_window)
                .is_some_and(|focus| focus.is::<gtk::Entry>() || focus.is::<gtk::SpinButton>());
            if !text_input_focused {
                match key {
                    gtk::gdk::Key::space => {
                        shortcut_play_pause.emit_clicked();
                        return glib::Propagation::Stop;
                    }
                    gtk::gdk::Key::Left => {
                        shortcut_previous.emit_clicked();
                        return glib::Propagation::Stop;
                    }
                    gtk::gdk::Key::Right => {
                        shortcut_next.emit_clicked();
                        return glib::Propagation::Stop;
                    }
                    gtk::gdk::Key::s | gtk::gdk::Key::S => {
                        shortcut_scan.emit_clicked();
                        return glib::Propagation::Stop;
                    }
                    gtk::gdk::Key::Return | gtk::gdk::Key::KP_Enter => {
                        shortcut_add_selected.emit_clicked();
                        return glib::Propagation::Stop;
                    }
                    gtk::gdk::Key::E if modifiers.contains(gtk::gdk::ModifierType::SHIFT_MASK) => {
                        shortcut_export.emit_clicked();
                        return glib::Propagation::Stop;
                    }
                    gtk::gdk::Key::I if modifiers.contains(gtk::gdk::ModifierType::SHIFT_MASK) => {
                        shortcut_import.emit_clicked();
                        return glib::Propagation::Stop;
                    }
                    _ => {}
                }
            }
        }
        if key == gtk::gdk::Key::Escape {
            if shortcut_active_search.borrow().is_some() {
                shortcut_clear.emit_clicked();
            } else {
                shortcut_catalog_list.unselect_all();
                shortcut_queue_list.unselect_all();
            }
            return glib::Propagation::Stop;
        }
        glib::Propagation::Proceed
    });
    window.add_controller(shortcut_controller);
    window.present();
}

enum UiEvent {
    Roots(Result<LibraryState, String>),
    RestoredLibrary(Result<(LibraryState, CatalogPage, u8, ServerClient), String>),
    HistoryCleared(
        Result<
            (
                LibraryState,
                CatalogPage,
                u8,
                ServerClient,
                Option<SearchCriteria>,
            ),
            String,
        >,
    ),
    Threshold(Result<u8, String>),
    StateExported(Result<PathBuf, String>),
    StateImported(Result<(LibraryState, CatalogPage, u8, QueueSnapshot), String>),
    StateReset(Result<(LibraryState, CatalogPage, u8, QueueSnapshot), String>),
    Recovery(Result<ServerProcess, String>),
    Catalog(
        Result<
            (
                ScanSummary,
                CatalogPage,
                ServerClient,
                Option<SearchCriteria>,
            ),
            String,
        >,
    ),
    Search {
        result: Result<(CatalogPage, ServerClient, SearchCriteria), String>,
        generation: u64,
    },
    CatalogPage(Result<CatalogPage, String>),
    Playback {
        state: Result<QueueSnapshot, String>,
        command_error: Option<String>,
    },
}

enum PlaybackRequest {
    Append(Vec<PathBuf>),
    AppendAllMatches(SearchCriteria),
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
    catalog_label: gtk::Label,
    clear_search: gtk::Button,
    add_root: gtk::Button,
    remove_root: gtk::Button,
    search_button: gtk::Button,
    add_selected: gtk::Button,
    add_all: gtk::Button,
    export_state: gtk::Button,
    import_state: gtk::Button,
    reset_state: gtk::Button,
    retry: gtk::Button,
    roots_model: Rc<RefCell<Vec<PathBuf>>>,
    catalog_entries: Rc<RefCell<Vec<LibraryEntry>>>,
    catalog_details: Rc<RefCell<Vec<LibraryEntry>>>,
    catalog_paging: Rc<RefCell<Option<(ServerClient, SearchCriteria, usize)>>>,
    active_search: Rc<RefCell<Option<SearchCriteria>>>,
    search_generation: Rc<Cell<u64>>,
    actions: CatalogActions,
    search_chips: gtk::Box,
    idle_panel: gtk::Box,
    results_panel: gtk::Box,
    no_matches: gtk::Label,
    status: gtk::Label,
    scan_button: gtk::Button,
    load_more: gtk::Button,
    played_threshold: gtk::SpinButton,
    syncing_threshold: Rc<Cell<bool>>,
    clear_all_history: gtk::Button,
    recovery_attempted: Rc<Cell<bool>>,
    queue: QueueUi,
    playback: PlaybackUi,
    poll_playback: Arc<AtomicBool>,
}

#[derive(Clone)]
struct CatalogActions {
    server_state: Rc<RefCell<ServerState>>,
    sender: Sender<UiEvent>,
    status: gtk::Label,
    active_search: Rc<RefCell<Option<SearchCriteria>>>,
}

#[derive(Clone)]
struct SearchForm {
    text: gtk::Entry,
    not_yet_played: gtk::DropDown,
    artist: gtk::Entry,
    artist_missing: gtk::CheckButton,
    album: gtk::Entry,
    album_missing: gtk::CheckButton,
    genre: gtk::Entry,
    genre_missing: gtk::CheckButton,
    year: gtk::Entry,
    year_missing: gtk::CheckButton,
    duration: gtk::Entry,
    location: gtk::Entry,
    play_count: gtk::Entry,
}

impl SearchForm {
    fn criteria(&self) -> Result<SearchCriteria, String> {
        let non_empty = |entry: &gtk::Entry| {
            let text = entry.text().trim().to_owned();
            (!text.is_empty()).then_some(text)
        };
        let text_filter = |entry: &gtk::Entry, missing: &gtk::CheckButton| {
            if missing.is_active() {
                Some(TextFilter::IsMissing)
            } else {
                non_empty(entry).map(TextFilter::Contains)
            }
        };
        let release_year = if self.year_missing.is_active() {
            Some(NumericFilter::IsMissing)
        } else {
            parse_numeric_filter::<u16>(&self.year, "release year")?
        };
        let criteria = SearchCriteria {
            text: non_empty(&self.text),
            not_yet_played: match self.not_yet_played.selected() {
                1 => Some(true),
                2 => Some(false),
                _ => None,
            },
            artist: text_filter(&self.artist, &self.artist_missing),
            album: text_filter(&self.album, &self.album_missing),
            genre: text_filter(&self.genre, &self.genre_missing),
            release_year,
            duration_seconds: parse_numeric_filter::<u64>(&self.duration, "duration")?,
            file_location: non_empty(&self.location),
            play_count: parse_numeric_filter::<u64>(&self.play_count, "play count")?,
        };
        criteria.validate()?;
        Ok(criteria)
    }

    fn reset(&self) {
        self.text.set_text("");
        self.not_yet_played.set_selected(0);
        self.artist.set_text("");
        self.artist_missing.set_active(false);
        self.album.set_text("");
        self.album_missing.set_active(false);
        self.genre.set_text("");
        self.genre_missing.set_active(false);
        self.year.set_text("");
        self.year_missing.set_active(false);
        self.duration.set_text("");
        self.location.set_text("");
        self.play_count.set_text("");
    }
}

fn replace_search_chips(chips: &gtk::Box, criteria: &SearchCriteria) {
    while let Some(child) = chips.first_child() {
        chips.remove(&child);
    }
    let mut labels = Vec::new();
    if let Some(text) = &criteria.text {
        labels.push(format!("Search: {text}"));
    }
    match criteria.not_yet_played {
        Some(true) => labels.push("Not-yet-played".to_owned()),
        Some(false) => labels.push("not Not-yet-played".to_owned()),
        None => {}
    }
    append_text_chip(&mut labels, "Artist", criteria.artist.as_ref());
    append_text_chip(&mut labels, "Album", criteria.album.as_ref());
    append_text_chip(&mut labels, "Genre", criteria.genre.as_ref());
    append_numeric_chip(&mut labels, "Year", criteria.release_year.as_ref());
    append_numeric_chip(
        &mut labels,
        "Duration (s)",
        criteria.duration_seconds.as_ref(),
    );
    if let Some(location) = &criteria.file_location {
        labels.push(format!("Location: {location}"));
    }
    append_numeric_chip(&mut labels, "Play count", criteria.play_count.as_ref());

    for full_label in &labels {
        let label = gtk::Label::new(Some(full_label));
        label.set_ellipsize(gtk::pango::EllipsizeMode::End);
        label.set_max_width_chars(28);
        label.set_margin_start(6);
        label.set_margin_end(6);
        label.set_margin_top(3);
        label.set_margin_bottom(3);
        label.set_tooltip_text(Some(full_label));
        label.update_property(&[gtk::accessible::Property::Label(full_label)]);
        let chip = gtk::Frame::new(None);
        chip.add_css_class("card");
        chip.set_child(Some(&label));
        chips.append(&chip);
    }
    chips.set_visible(!labels.is_empty());
}

fn append_text_chip(labels: &mut Vec<String>, name: &str, filter: Option<&TextFilter>) {
    match filter {
        Some(TextFilter::Contains(text)) => labels.push(format!("{name}: {text}")),
        Some(TextFilter::IsMissing) => labels.push(format!("{name} is missing")),
        None => {}
    }
}

fn append_numeric_chip<T: std::fmt::Display>(
    labels: &mut Vec<String>,
    name: &str,
    filter: Option<&NumericFilter<T>>,
) {
    match filter {
        Some(NumericFilter::Exact(value)) => labels.push(format!("{name}: {value}")),
        Some(NumericFilter::RangeInclusive { min, max }) => {
            labels.push(format!("{name}: {min}..{max}"));
        }
        Some(NumericFilter::IsMissing) => labels.push(format!("{name} is missing")),
        None => {}
    }
}

fn parse_numeric_filter<T>(
    entry: &gtk::Entry,
    label: &str,
) -> Result<Option<NumericFilter<T>>, String>
where
    T: Ord + std::str::FromStr,
{
    let text = entry.text();
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    if let Some((min, max)) = text.split_once("..") {
        let min = min
            .trim()
            .parse::<T>()
            .map_err(|_| format!("Enter a valid lower {label} bound."))?;
        let max = max
            .trim()
            .parse::<T>()
            .map_err(|_| format!("Enter a valid upper {label} bound."))?;
        if min > max {
            return Err(format!(
                "The lower {label} bound must not exceed the upper bound."
            ));
        }
        Ok(Some(NumericFilter::RangeInclusive { min, max }))
    } else {
        text.parse::<T>()
            .map(NumericFilter::Exact)
            .map(Some)
            .map_err(|_| format!("Enter a valid {label} or inclusive range."))
    }
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
    current_track_subtitle: gtk::Label,
    position: gtk::Scale,
    elapsed: gtk::Label,
    duration: gtk::Label,
    toggle: gtk::Button,
    toggle_icon: gtk::Image,
    stop: gtk::Button,
}

fn install_library_events(receiver: Receiver<UiEvent>, ui: LibraryUi) {
    let LibraryUi {
        roots_list,
        catalog_list,
        catalog_label,
        clear_search,
        add_root,
        remove_root,
        search_button,
        add_selected,
        add_all,
        export_state,
        import_state,
        reset_state,
        retry,
        roots_model,
        catalog_entries,
        catalog_details,
        catalog_paging,
        active_search,
        search_generation,
        actions,
        search_chips,
        idle_panel,
        results_panel,
        no_matches,
        status,
        scan_button,
        load_more,
        played_threshold,
        syncing_threshold,
        clear_all_history,
        recovery_attempted,
        queue,
        playback,
        poll_playback,
    } = ui;
    glib::timeout_add_local(Duration::from_millis(40), move || {
        while let Ok(event) = receiver.try_recv() {
            match event {
                UiEvent::RestoredLibrary(Ok((state, page, threshold, client))) => {
                    *roots_model.borrow_mut() = state.selected_roots.clone();
                    replace_root_rows(&roots_list, &state.selected_roots);
                    remember_catalog_entries(&catalog_details, &page.entries);
                    replace_catalog_rows(&catalog_list, &page.entries, &catalog_entries, &actions);
                    active_search.borrow_mut().take();
                    *catalog_paging.borrow_mut() = page
                        .next_offset
                        .map(|offset| (client, SearchCriteria::default(), offset));
                    replace_search_chips(&search_chips, &SearchCriteria::default());
                    no_matches.set_visible(false);
                    catalog_label.set_text("Library");
                    add_all.set_label("Add All Tracks");
                    load_more.set_label("Load More Tracks");
                    load_more.set_visible(page.next_offset.is_some());
                    load_more.set_sensitive(page.next_offset.is_some());
                    results_panel.set_visible(state.catalog_entries > 0);
                    idle_panel.set_visible(state.catalog_entries == 0);
                    clear_all_history.set_sensitive(state.catalog_entries > 0);
                    syncing_threshold.set(true);
                    played_threshold.set_value(f64::from(threshold));
                    syncing_threshold.set(false);
                    status.set_text(&format!(
                        "Restored {} cached tracks. Scan Library to refresh.",
                        state.catalog_entries
                    ));
                }
                UiEvent::RestoredLibrary(Err(error)) => {
                    status.set_text(&format!("Could not restore library: {error}"));
                }
                UiEvent::HistoryCleared(Ok((state, page, threshold, client, criteria))) => {
                    *roots_model.borrow_mut() = state.selected_roots.clone();
                    replace_root_rows(&roots_list, &state.selected_roots);
                    remember_catalog_entries(&catalog_details, &page.entries);
                    replace_catalog_rows(&catalog_list, &page.entries, &catalog_entries, &actions);
                    if let Some(criteria) = criteria {
                        *active_search.borrow_mut() = Some(criteria.clone());
                        replace_search_chips(&search_chips, &criteria);
                        *catalog_paging.borrow_mut() =
                            page.next_offset.map(|offset| (client, criteria, offset));
                        results_panel.set_visible(true);
                        idle_panel.set_visible(false);
                        no_matches.set_visible(page.entries.is_empty());
                        catalog_label.set_text("Matching Tracks");
                        add_all.set_label("Add All Matches");
                        load_more.set_label("Load More Matches");
                        load_more.set_visible(page.next_offset.is_some());
                        load_more.set_sensitive(page.next_offset.is_some());
                    } else {
                        active_search.borrow_mut().take();
                        *catalog_paging.borrow_mut() = page
                            .next_offset
                            .map(|offset| (client, SearchCriteria::default(), offset));
                        replace_search_chips(&search_chips, &SearchCriteria::default());
                        no_matches.set_visible(false);
                        catalog_label.set_text("Library");
                        add_all.set_label("Add All Tracks");
                        load_more.set_label("Load More Tracks");
                        load_more.set_visible(page.next_offset.is_some());
                        load_more.set_sensitive(page.next_offset.is_some());
                        results_panel.set_visible(state.catalog_entries > 0);
                        idle_panel.set_visible(state.catalog_entries == 0);
                    }
                    clear_all_history.set_sensitive(state.catalog_entries > 0);
                    syncing_threshold.set(true);
                    played_threshold.set_value(f64::from(threshold));
                    syncing_threshold.set(false);
                    status.set_text("Play history cleared.");
                }
                UiEvent::HistoryCleared(Err(error)) => {
                    status.set_text(&format!("Could not clear play history: {error}"));
                }
                UiEvent::Threshold(Ok(_)) => {}
                UiEvent::Threshold(Err(error)) => {
                    status.set_text(&format!("Could not save played threshold: {error}"));
                }
                UiEvent::StateExported(Ok(path)) => {
                    status.set_text(&format!("State exported to {}.", path.display()));
                }
                UiEvent::StateExported(Err(error)) => {
                    status.set_text(&format!("Could not export state: {error}"));
                }
                UiEvent::StateImported(Ok((state, page, threshold, snapshot))) => {
                    clear_search.emit_clicked();
                    *roots_model.borrow_mut() = state.selected_roots.clone();
                    replace_root_rows(&roots_list, &state.selected_roots);
                    remember_catalog_entries(&catalog_details, &page.entries);
                    replace_catalog_rows(&catalog_list, &page.entries, &catalog_entries, &actions);
                    catalog_paging.borrow_mut().take();
                    results_panel.set_visible(state.catalog_entries > 0);
                    idle_panel.set_visible(state.catalog_entries == 0);
                    no_matches.set_visible(false);
                    catalog_label.set_text("Library");
                    add_all.set_label("Add All Tracks");
                    if let Some(client) = actions.server_state.borrow().client() {
                        *catalog_paging.borrow_mut() = page
                            .next_offset
                            .map(|offset| (client, SearchCriteria::default(), offset));
                    }
                    load_more.set_label("Load More Tracks");
                    load_more.set_visible(page.next_offset.is_some());
                    load_more.set_sensitive(page.next_offset.is_some());
                    clear_all_history.set_sensitive(state.catalog_entries > 0);
                    syncing_threshold.set(true);
                    played_threshold.set_value(f64::from(threshold));
                    syncing_threshold.set(false);
                    for update in &snapshot.play_count_updates {
                        apply_play_count_update(
                            &catalog_list,
                            &catalog_entries,
                            &catalog_details,
                            update,
                            &actions,
                        );
                    }
                    playback.apply(&snapshot.playback, &catalog_details.borrow(), &status);
                    queue.apply(&snapshot, &catalog_details.borrow());
                    poll_playback.store(
                        matches!(
                            snapshot.playback.status,
                            PlaybackStatus::Playing | PlaybackStatus::Paused
                        ),
                        Ordering::Relaxed,
                    );
                    status.set_text(&format!(
                        "State imported. Restored {} cached tracks.",
                        state.catalog_entries
                    ));
                }
                UiEvent::StateImported(Err(error)) => {
                    status.set_text(&format!("Could not import state: {error}"));
                }
                UiEvent::StateReset(Ok((state, page, threshold, snapshot))) => {
                    clear_search.emit_clicked();
                    *roots_model.borrow_mut() = state.selected_roots.clone();
                    replace_root_rows(&roots_list, &state.selected_roots);
                    remember_catalog_entries(&catalog_details, &page.entries);
                    replace_catalog_rows(&catalog_list, &page.entries, &catalog_entries, &actions);
                    catalog_paging.borrow_mut().take();
                    results_panel.set_visible(state.catalog_entries > 0);
                    idle_panel.set_visible(state.catalog_entries == 0);
                    no_matches.set_visible(false);
                    catalog_label.set_text("Library");
                    add_all.set_label("Add All Tracks");
                    if let Some(client) = actions.server_state.borrow().client() {
                        *catalog_paging.borrow_mut() = page
                            .next_offset
                            .map(|offset| (client, SearchCriteria::default(), offset));
                    }
                    load_more.set_label("Load More Tracks");
                    load_more.set_visible(page.next_offset.is_some());
                    load_more.set_sensitive(page.next_offset.is_some());
                    clear_all_history.set_sensitive(state.catalog_entries > 0);
                    syncing_threshold.set(true);
                    played_threshold.set_value(f64::from(threshold));
                    syncing_threshold.set(false);
                    playback.apply(&snapshot.playback, &catalog_details.borrow(), &status);
                    queue.apply(&snapshot, &catalog_details.borrow());
                    poll_playback.store(false, Ordering::Relaxed);
                    status.set_text("App state reset. Media files were not changed.");
                }
                UiEvent::StateReset(Err(error)) => {
                    status.set_text(&format!("Could not reset app state: {error}"));
                }
                UiEvent::Recovery(Ok(process)) => {
                    let client = process.client();
                    {
                        let mut server_state = actions.server_state.borrow_mut();
                        if let Some(previous) = server_state.process.as_mut() {
                            let _ = previous.is_running();
                        }
                        server_state.process = Some(process);
                    }
                    recovery_attempted.set(false);
                    retry.set_sensitive(false);
                    set_server_controls_sensitive(
                        &add_root,
                        &remove_root,
                        &scan_button,
                        roots_list.selected_row().is_some(),
                        true,
                    );
                    search_button.set_sensitive(true);
                    add_all.set_sensitive(true);
                    export_state.set_sensitive(true);
                    import_state.set_sensitive(true);
                    reset_state.set_sensitive(true);
                    played_threshold.set_sensitive(true);
                    queue.set_available(true);
                    status.set_text("Server restarted; restoring the Library and Queue…");
                    let sender = actions.sender.clone();
                    thread::spawn(move || {
                        let _ =
                            sender.send(UiEvent::RestoredLibrary(read_restored_library(&client)));
                        let _ = sender.send(UiEvent::Playback {
                            state: client.queue_state().map_err(|error| error.to_string()),
                            command_error: None,
                        });
                    });
                }
                UiEvent::Recovery(Err(error)) => {
                    status.set_text(&format!(
                        "Automatic server recovery failed: {error}. Select Retry or Quit."
                    ));
                    retry.set_sensitive(true);
                }
                UiEvent::Search { result, generation } => {
                    if generation != search_generation.get() {
                        continue;
                    }
                    match result {
                        Ok((page, client, criteria)) => {
                            let library_view = criteria == SearchCriteria::default();
                            if library_view {
                                active_search.borrow_mut().take();
                            } else {
                                *active_search.borrow_mut() = Some(criteria.clone());
                            }
                            replace_search_chips(&search_chips, &criteria);
                            remember_catalog_entries(&catalog_details, &page.entries);
                            replace_catalog_rows(
                                &catalog_list,
                                &page.entries,
                                &catalog_entries,
                                &actions,
                            );
                            *catalog_paging.borrow_mut() = page
                                .next_offset
                                .map(|offset| (client, criteria.clone(), offset));
                            no_matches.set_visible(!library_view && page.entries.is_empty());
                            results_panel.set_visible(!library_view || !page.entries.is_empty());
                            idle_panel.set_visible(library_view && page.entries.is_empty());
                            load_more.set_visible(page.next_offset.is_some());
                            load_more.set_sensitive(page.next_offset.is_some());
                            if library_view {
                                catalog_label.set_text("Library");
                                add_all.set_label("Add All Tracks");
                                load_more.set_label("Load More Tracks");
                                status.set_text("Library loaded.");
                            } else {
                                catalog_label.set_text("Matching Tracks");
                                add_all.set_label("Add All Matches");
                                load_more.set_label("Load More Matches");
                                status.set_text("Search results updated.");
                            }
                        }
                        Err(error) => status.set_text(&format!("Search failed: {error}")),
                    }
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
                UiEvent::Catalog(Ok((summary, page, client, criteria))) => {
                    remember_catalog_entries(&catalog_details, &page.entries);
                    replace_catalog_rows(&catalog_list, &page.entries, &catalog_entries, &actions);
                    scan_button.set_sensitive(true);
                    status.set_text(&scan_status_text(&summary));
                    if let Some(criteria) = criteria {
                        *active_search.borrow_mut() = Some(criteria.clone());
                        replace_search_chips(&search_chips, &criteria);
                        *catalog_paging.borrow_mut() =
                            page.next_offset.map(|offset| (client, criteria, offset));
                        results_panel.set_visible(true);
                        idle_panel.set_visible(false);
                        no_matches.set_visible(page.entries.is_empty());
                        catalog_label.set_text("Matching Tracks");
                        add_all.set_label("Add All Matches");
                        load_more.set_label("Load More Matches");
                        load_more.set_visible(page.next_offset.is_some());
                        load_more.set_sensitive(page.next_offset.is_some());
                    } else {
                        active_search.borrow_mut().take();
                        *catalog_paging.borrow_mut() = page
                            .next_offset
                            .map(|offset| (client, SearchCriteria::default(), offset));
                        replace_search_chips(&search_chips, &SearchCriteria::default());
                        no_matches.set_visible(false);
                        catalog_label.set_text("Library");
                        add_all.set_label("Add All Tracks");
                        load_more.set_label("Load More Tracks");
                        load_more.set_visible(page.next_offset.is_some());
                        load_more.set_sensitive(page.next_offset.is_some());
                        let has_entries = summary.active_entries + summary.inactive_entries > 0;
                        results_panel.set_visible(has_entries);
                        idle_panel.set_visible(!has_entries);
                    }
                    clear_all_history
                        .set_sensitive(summary.active_entries + summary.inactive_entries > 0);
                }
                UiEvent::Catalog(Err(error)) => {
                    scan_button.set_sensitive(true);
                    status.set_text(&format!("Library scan failed: {error}"));
                }
                UiEvent::CatalogPage(Ok(page)) => {
                    remember_catalog_entries(&catalog_details, &page.entries);
                    append_catalog_rows(&catalog_list, &page.entries, &catalog_entries, &actions);
                    let search = catalog_paging.borrow().clone();
                    *catalog_paging.borrow_mut() = search.and_then(|(client, criteria, _)| {
                        page.next_offset.map(|offset| (client, criteria, offset))
                    });
                    load_more.set_visible(page.next_offset.is_some());
                    load_more.set_sensitive(page.next_offset.is_some());
                    status.set_text("More tracks loaded.");
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
                            for update in &snapshot.play_count_updates {
                                apply_play_count_update(
                                    &catalog_list,
                                    &catalog_entries,
                                    &catalog_details,
                                    update,
                                    &actions,
                                );
                            }
                            if !snapshot.play_count_updates.is_empty()
                                && let Some(criteria) = active_search.borrow().clone()
                                && let Some(client) = actions.server_state.borrow().client()
                            {
                                let generation = search_generation.get().wrapping_add(1);
                                search_generation.set(generation);
                                send_search(client, criteria, generation, actions.sender.clone());
                            }
                            playback.apply(&snapshot.playback, &catalog_details.borrow(), &status);
                            queue.apply(&snapshot, &catalog_details.borrow());
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
                            playback
                                .current_track
                                .set_text("Playback server unavailable");
                            playback.current_track_subtitle.set_text("");
                            playback.toggle.set_sensitive(false);
                            playback.stop.set_sensitive(false);
                            retry.set_sensitive(false);
                            set_server_controls_sensitive(
                                &add_root,
                                &remove_root,
                                &scan_button,
                                false,
                                false,
                            );
                            search_button.set_sensitive(false);
                            add_selected.set_sensitive(false);
                            add_all.set_sensitive(false);
                            export_state.set_sensitive(false);
                            import_state.set_sensitive(false);
                            reset_state.set_sensitive(false);
                            played_threshold.set_sensitive(false);
                            clear_all_history.set_sensitive(false);
                            queue.set_available(false);
                            if !recovery_attempted.replace(true) {
                                status.set_text(&format!(
                                    "Playback server stopped. Attempting one automatic restart: {error}"
                                ));
                                let (executable, socket_path, library_state_path) = {
                                    let state = actions.server_state.borrow();
                                    (
                                        state.executable.clone(),
                                        state.socket_path.clone(),
                                        state.library_state_path.clone(),
                                    )
                                };
                                let sender = actions.sender.clone();
                                thread::spawn(move || {
                                    let result = ServerProcess::start_with_library_state(
                                        &executable,
                                        &socket_path,
                                        &library_state_path,
                                        Duration::from_secs(2),
                                    )
                                    .map_err(|error| error.to_string());
                                    let _ = sender.send(UiEvent::Recovery(result));
                                });
                            } else {
                                status.set_text(&format!(
                                    "Automatic recovery already attempted: {error}. Select Retry or Quit."
                                ));
                                retry.set_sensitive(true);
                            }
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
        PlaybackRequest::AppendAllMatches(criteria) => {
            let mut offset = 0;
            let mut tracks = Vec::new();
            loop {
                let page = client.search_catalog(&criteria, offset, 32)?;
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
            || None,
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
                        let release = view
                            .release_title
                            .unwrap_or_else(|| "No album tag".to_owned());
                        (title.to_owned(), format!("{artists} · {release}"))
                    })
                    .or_else(|| {
                        Some((
                            path.file_name().map_or_else(
                                || path.display().to_string(),
                                |name| name.to_string_lossy().into_owned(),
                            ),
                            path.display().to_string(),
                        ))
                    })
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
        if let Some((title, subtitle)) = track_detail {
            self.current_track.set_text(&title);
            self.current_track_subtitle.set_text(&subtitle);
        } else {
            self.current_track.set_text("Nothing is playing");
            self.current_track_subtitle.set_text("");
        }
        self.elapsed.set_text(&format_time(snapshot.position_ms));
        self.duration.set_text(
            &snapshot
                .duration_ms
                .map(format_time)
                .unwrap_or_else(|| "0:00".to_owned()),
        );
        let duration_ms = snapshot.duration_ms.unwrap_or(1).max(1);
        self.position.set_range(0.0, duration_ms as f64);
        self.position
            .set_value(snapshot.position_ms.min(duration_ms) as f64);
        let (icon_name, label) = if snapshot.status == PlaybackStatus::Playing {
            ("media-playback-pause-symbolic", "Pause playback")
        } else {
            ("media-playback-start-symbolic", "Play playback")
        };
        self.toggle_icon.set_icon_name(Some(icon_name));
        self.toggle.set_tooltip_text(Some(label));
        self.toggle
            .update_property(&[gtk::accessible::Property::Label(label)]);
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
    actions: &CatalogActions,
) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }
    catalog_entries.borrow_mut().clear();
    append_catalog_rows(list, entries, catalog_entries, actions);
}

fn remember_catalog_entries(
    catalog_details: &Rc<RefCell<Vec<LibraryEntry>>>,
    entries: &[LibraryEntry],
) {
    let mut details = catalog_details.borrow_mut();
    for entry in entries {
        if let Some(existing) = details
            .iter_mut()
            .find(|existing| existing.path == entry.path)
        {
            *existing = entry.clone();
        } else {
            details.push(entry.clone());
        }
    }
}

fn append_catalog_rows(
    list: &gtk::ListBox,
    entries: &[LibraryEntry],
    catalog_entries: &Rc<RefCell<Vec<LibraryEntry>>>,
    actions: &CatalogActions,
) {
    for entry in entries {
        list.append(&render_catalog_row(entry, actions));
    }
    catalog_entries.borrow_mut().extend_from_slice(entries);
}

fn render_catalog_row(entry: &LibraryEntry, actions: &CatalogActions) -> gtk::ListBoxRow {
    let view = present_catalog_entry(entry);
    let row = gtk::ListBoxRow::new();
    row.set_selectable(view.selectable);
    row.set_activatable(view.selectable);

    let track_content = gtk::Box::new(gtk::Orientation::Vertical, 2);
    track_content.set_hexpand(true);
    let title = view.title.as_deref().unwrap_or("(No title tag)");
    let title = if view.active {
        title.to_owned()
    } else {
        format!("Unavailable — {title}")
    };
    let title_label = gtk::Label::new(Some(&title));
    title_label.set_halign(gtk::Align::Start);
    title_label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    title_label.set_width_chars(18);
    title_label.add_css_class("heading");
    track_content.append(&title_label);

    let artist = if view.artists.is_empty() {
        "(No artist tag)".to_owned()
    } else {
        view.artists.join(", ")
    };
    let artist_label = gtk::Label::new(Some(&artist));
    artist_label.set_halign(gtk::Align::Start);
    artist_label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    artist_label.set_width_chars(18);
    artist_label.add_css_class("dim-label");
    track_content.append(&artist_label);

    let release = view.release_title.as_deref().unwrap_or("(No album tag)");
    let release = match view.release_year {
        Some(year) => format!("{release} · {year}"),
        None => release.to_owned(),
    };
    let release_label = gtk::Label::new(Some(&release));
    release_label.set_halign(gtk::Align::Start);
    release_label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    release_label.set_width_chars(14);

    let duration = view
        .duration
        .map(|duration| format!("{}:{:02}", duration.as_secs() / 60, duration.as_secs() % 60))
        .unwrap_or_else(|| "—".to_owned());
    let duration_label = gtk::Label::new(Some(&duration));
    duration_label.set_halign(gtk::Align::Start);
    duration_label.set_width_chars(6);

    let play_count_label = gtk::Label::new(Some(&entry.play_count.to_string()));
    play_count_label.set_halign(gtk::Align::Start);
    play_count_label.set_width_chars(5);
    play_count_label.add_css_class("dim-label");
    let play_count_description = if entry.play_count == 0 {
        "Not-yet-played (0 qualifying plays)".to_owned()
    } else if entry.play_count == 1 {
        "Played once (1 qualifying play)".to_owned()
    } else {
        format!("Played {} times (qualifying plays)", entry.play_count)
    };
    play_count_label.set_tooltip_text(Some(&play_count_description));

    let row_content = gtk::Grid::new();
    row_content.set_hexpand(true);
    row_content.set_column_spacing(12);
    row_content.set_margin_start(8);
    row_content.set_margin_end(8);
    row_content.set_margin_top(4);
    row_content.set_margin_bottom(4);
    row_content.attach(&track_content, 0, 0, 1, 1);
    row_content.attach(&release_label, 1, 0, 1, 1);
    row_content.attach(&duration_label, 2, 0, 1, 1);
    row_content.attach(&play_count_label, 3, 0, 1, 1);

    let genres = if view.genres.is_empty() {
        "(No genre tag)".to_owned()
    } else {
        view.genres.join(" · ")
    };
    let track_number = view
        .track_number
        .map(|number| format!("Track {number}"))
        .unwrap_or_else(|| "Track number unavailable".to_owned());
    let tooltip = format!(
        "{artist} · {release} · {genres} · {track_number}\n{}\n{} qualifying plays",
        view.location.display(),
        entry.play_count
    );
    row.set_tooltip_text(Some(&tooltip));

    if entry.play_count > 0 {
        let clear_history = gtk::Button::with_label("Clear history");
        clear_history.set_valign(gtk::Align::Center);
        let track_name = view.title.as_deref().unwrap_or("untitled track");
        let clear_label = format!("Clear play history for {track_name}");
        clear_history.update_property(&[gtk::accessible::Property::Label(&clear_label)]);
        let clear_path = entry.path.clone();
        let clear_actions = actions.clone();
        clear_history.connect_clicked(move |_| {
            if let Some(client) = clear_actions.server_state.borrow().client() {
                let criteria = clear_actions.active_search.borrow().clone();
                send_clear_entry_history(
                    client,
                    clear_path.clone(),
                    criteria,
                    clear_actions.sender.clone(),
                );
            } else {
                clear_actions
                    .status
                    .set_text("Server unavailable. Select Retry.");
            }
        });
        row_content.attach(&clear_history, 4, 0, 1, 1);
    } else {
        let history_placeholder = gtk::Label::new(None);
        history_placeholder.set_width_chars(12);
        row_content.attach(&history_placeholder, 4, 0, 1, 1);
    }

    row.set_child(Some(&row_content));
    row
}

fn apply_play_count_update(
    list: &gtk::ListBox,
    catalog_entries: &Rc<RefCell<Vec<LibraryEntry>>>,
    catalog_details: &Rc<RefCell<Vec<LibraryEntry>>>,
    update: &PlayCountUpdate,
    actions: &CatalogActions,
) {
    if let Some(entry) = catalog_details
        .borrow_mut()
        .iter_mut()
        .find(|entry| entry.path == update.track)
    {
        entry.play_count = update.play_count;
    }
    let (index, updated_entry) = {
        let mut entries = catalog_entries.borrow_mut();
        let Some(index) = entries.iter().position(|entry| entry.path == update.track) else {
            return;
        };
        entries[index].play_count = update.play_count;
        (index, entries[index].clone())
    };
    let was_selected = list
        .row_at_index(index as i32)
        .is_some_and(|row| row.is_selected());
    if let Some(row) = list.row_at_index(index as i32) {
        list.remove(&row);
        list.insert(&render_catalog_row(&updated_entry, actions), index as i32);
        if was_selected {
            list.select_row(list.row_at_index(index as i32).as_ref());
        }
    }
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

fn send_threshold_operation(client: ServerClient, percent: u8, sender: Sender<UiEvent>) {
    thread::spawn(move || {
        let result = client
            .set_played_threshold(percent)
            .map_err(|error| error.to_string());
        let _ = sender.send(UiEvent::Threshold(result));
    });
}

fn send_export_state(client: ServerClient, path: PathBuf, sender: Sender<UiEvent>) {
    thread::spawn(move || {
        let result = client
            .export_app_state(&path)
            .map(|()| path)
            .map_err(|error| error.to_string());
        let _ = sender.send(UiEvent::StateExported(result));
    });
}

fn send_import_state(client: ServerClient, path: PathBuf, sender: Sender<UiEvent>) {
    thread::spawn(move || {
        let result = client
            .import_app_state(&path)
            .map_err(|error| error.to_string())
            .and_then(|_| {
                let (state, page, threshold, client, _) = read_library_view(&client, None)?;
                let queue = client.queue_state().map_err(|error| error.to_string())?;
                Ok((state, page, threshold, queue))
            });
        let _ = sender.send(UiEvent::StateImported(result));
    });
}

fn send_reset_app_state(client: ServerClient, sender: Sender<UiEvent>) {
    thread::spawn(move || {
        let result = client
            .reset_app_state()
            .map_err(|error| error.to_string())
            .and_then(|_| {
                let (state, page, threshold, client, _) = read_library_view(&client, None)?;
                let queue = client.queue_state().map_err(|error| error.to_string())?;
                Ok((state, page, threshold, queue))
            });
        let _ = sender.send(UiEvent::StateReset(result));
    });
}

fn send_clear_entry_history(
    client: ServerClient,
    path: PathBuf,
    criteria: Option<SearchCriteria>,
    sender: Sender<UiEvent>,
) {
    thread::spawn(move || {
        let result = client
            .clear_entry_play_history(&path)
            .map_err(|error| error.to_string())
            .and_then(|_| read_library_view(&client, criteria));
        let _ = sender.send(UiEvent::HistoryCleared(result));
    });
}

fn send_clear_all_history(
    client: ServerClient,
    criteria: Option<SearchCriteria>,
    sender: Sender<UiEvent>,
) {
    thread::spawn(move || {
        let result = client
            .clear_all_play_history()
            .map_err(|error| error.to_string())
            .and_then(|_| read_library_view(&client, criteria));
        let _ = sender.send(UiEvent::HistoryCleared(result));
    });
}

fn read_scanned_catalog(
    client: &ServerClient,
    criteria: Option<SearchCriteria>,
) -> Result<
    (
        ScanSummary,
        CatalogPage,
        ServerClient,
        Option<SearchCriteria>,
    ),
    String,
> {
    let summary = client.scan_library().map_err(|error| error.to_string())?;
    let all_tracks = SearchCriteria::default();
    let page_criteria = criteria.as_ref().unwrap_or(&all_tracks);
    let page = client
        .search_catalog(page_criteria, 0, 32)
        .map_err(|error| error.to_string())?;
    Ok((summary, page, client.clone(), criteria))
}

fn send_search(
    client: ServerClient,
    criteria: SearchCriteria,
    generation: u64,
    sender: Sender<UiEvent>,
) {
    thread::spawn(move || {
        let result = client
            .search_catalog(&criteria, 0, 32)
            .map_err(|error| error.to_string())
            .map(|page| (page, client.clone(), criteria));
        let _ = sender.send(UiEvent::Search { result, generation });
    });
}

fn read_restored_library(
    client: &ServerClient,
) -> Result<(LibraryState, CatalogPage, u8, ServerClient), String> {
    read_library_view(client, None)
        .map(|(state, page, threshold, client, _)| (state, page, threshold, client))
}

fn read_library_view(
    client: &ServerClient,
    criteria: Option<SearchCriteria>,
) -> Result<
    (
        LibraryState,
        CatalogPage,
        u8,
        ServerClient,
        Option<SearchCriteria>,
    ),
    String,
> {
    let state = client.library_state().map_err(|error| error.to_string())?;
    let all_tracks = SearchCriteria::default();
    let page_criteria = criteria.as_ref().unwrap_or(&all_tracks);
    let page = client
        .search_catalog(page_criteria, 0, 32)
        .map_err(|error| error.to_string())?;
    let threshold = client
        .played_threshold()
        .map_err(|error| error.to_string())?;
    Ok((state, page, threshold, client.clone(), criteria))
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

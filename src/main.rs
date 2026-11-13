#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod bue;
mod library;

use std::{
    cell::RefCell, fs::OpenOptions, path::PathBuf, rc::Rc, sync::mpsc, thread, time::Duration,
};

use anyhow::{Context, Result};
use bue::{Credentials, Expired, Portal};
use directories::ProjectDirs;
use library::{Course, Library, SavedFile};
use slint::winit_030::{
    winit::event::{ElementState, MouseButton, WindowEvent},
    EventResult, WinitWindowAccessor,
};
use slint::{ComponentHandle, ModelRc, VecModel};

slint::include_modules!();

const SYNC_INTERVAL: Duration = Duration::from_secs(15 * 60);

enum Request {
    Refresh,
    Connect(Credentials),
    Disconnect,
}

#[derive(Default)]
struct View {
    courses: Vec<Course>,
    selected: Option<i64>,
    files: Vec<SavedFile>,
}

fn main() -> Result<()> {
    let ui = AppWindow::new()?;
    let root = ProjectDirs::from("", "", "Elearnin")
        .context("Could not locate your application data directory")?
        .data_local_dir()
        .to_owned();
    let library = match Library::open(&root) {
        Ok(library) => Rc::new(library),
        Err(error) => {
            ui.set_status(format!("Could not open the local library. {error}").into());
            ui.run()?;
            return Ok(());
        }
    };
    let view = Rc::new(RefCell::new(View::default()));
    ui.set_dark_mode(library.dark_mode().unwrap_or_default());
    let weak = ui.as_weak();
    let preferences = Rc::clone(&library);
    ui.on_toggle_theme(move || {
        if let Some(ui) = weak.upgrade() {
            let dark = !ui.get_dark_mode();
            match preferences.set_dark_mode(dark) {
                Ok(()) => ui.set_dark_mode(dark),
                Err(error) => ui.set_status(format!("Could not save appearance. {error}").into()),
            }
        }
    });
    redraw(&ui, &library, &mut view.borrow_mut());
    ui.set_status("Looking for saved BUE sign-in…".into());

    let (requests, incoming) = mpsc::channel();
    let (reports, updates) = mpsc::channel::<Report>();
    let weak = ui.as_weak();
    let update_library = Rc::clone(&library);
    let update_view = Rc::clone(&view);
    ui.on_read_sync_status(move || {
        let Some(ui) = weak.upgrade() else { return };
        while let Ok(report) = updates.try_recv() {
            ui.set_busy(report.busy);
            ui.set_connected(report.connected);
            ui.set_status(report.message.into());
            if report.refresh_library {
                redraw(&ui, &update_library, &mut update_view.borrow_mut());
            }
            if report.open_modules && ui.get_page() == "settings" {
                ui.invoke_home();
            }
        }
    });
    let signal = ui.as_weak();
    let sync_root = root.clone();
    thread::spawn(move || worker(sync_root, incoming, reports, signal));
    ui.set_busy(true);
    requests.send(Request::Refresh)?;

    let weak = ui.as_weak();
    let choose_library = Rc::clone(&library);
    let choose_view = Rc::clone(&view);
    ui.on_choose(move |index| {
        let Some(ui) = weak.upgrade() else { return };
        let Ok(index) = usize::try_from(index) else {
            return;
        };
        let mut view = choose_view.borrow_mut();
        if ui.get_page() == "modules" {
            if let Some(course) = view.courses.get(index) {
                view.selected = Some(course.id);
                ui.set_page("course".into());
                redraw(&ui, &choose_library, &mut view);
            }
        } else if ui.get_page() == "course" {
            if let (Some(course), Some(file)) = (view.selected, view.files.get(index)) {
                let file = file.clone();
                let root = root.clone();
                let ui = ui.as_weak();
                thread::spawn(move || {
                    if let Err(error) =
                        open_saved(&root, course, &file, |path| open::that_detached(path))
                    {
                        let message = error.to_string();
                        let _ = ui.upgrade_in_event_loop(move |ui| ui.set_status(message.into()));
                    }
                });
            }
        }
    });

    let weak = ui.as_weak();
    ui.on_home(move || {
        if let Some(ui) = weak.upgrade() {
            ui.set_page("modules".into());
            let mut view = view.borrow_mut();
            view.selected = None;
            redraw(&ui, &library, &mut view);
        }
    });
    let weak = ui.as_weak();
    ui.window().on_winit_window_event(move |_, event| {
        if let WindowEvent::MouseInput {
            state: ElementState::Pressed,
            button: MouseButton::Back,
            ..
        } = event
        {
            if let Some(ui) = weak.upgrade().filter(|ui| ui.get_page() == "course") {
                ui.invoke_home();
                return EventResult::PreventDefault;
            }
        }
        EventResult::Propagate
    });
    let weak = ui.as_weak();
    let refresh = requests.clone();
    ui.on_refresh(move || submit(&weak, &refresh, Request::Refresh));
    let weak = ui.as_weak();
    let connect = requests.clone();
    ui.on_connect(move |username, password| {
        let username = username.trim().to_owned();
        if !username.is_empty() && !password.is_empty() {
            submit(
                &weak,
                &connect,
                Request::Connect(Credentials {
                    username,
                    password: password.to_string(),
                }),
            );
        }
    });
    let weak = ui.as_weak();
    ui.on_disconnect(move || submit(&weak, &requests, Request::Disconnect));
    ui.run()?;
    Ok(())
}

fn submit(ui: &slint::Weak<AppWindow>, requests: &mpsc::Sender<Request>, request: Request) {
    if let Some(ui) = ui.upgrade() {
        if !ui.get_busy() {
            if requests.send(request).is_ok() {
                ui.set_busy(true);
            } else {
                ui.set_status("The sync service has stopped. Reopen Elearnin’ to retry.".into());
            }
        }
    }
}

fn redraw(ui: &AppWindow, library: &Library, view: &mut View) {
    if let Some(id) = view.selected {
        match library.files(id) {
            Ok(files) => {
                // Keep the same clickable rows if sync only changed their on-disk copies.
                if files.len() != view.files.len()
                    || files.iter().zip(&view.files).any(|(new, old)| {
                        (&new.id, &new.section, &new.title, &new.filename)
                            != (&old.id, &old.section, &old.title, &old.filename)
                    })
                {
                    let [lectures, tutorials, other] = material_columns(&files);
                    ui.set_lectures(ModelRc::new(VecModel::from(lectures)));
                    ui.set_tutorials(ModelRc::new(VecModel::from(tutorials)));
                    ui.set_other(ModelRc::new(VecModel::from(other)));
                }
                view.files = files;
            }
            Err(error) => {
                view.files.clear();
                ui.set_lectures(ModelRc::default());
                ui.set_tutorials(ModelRc::default());
                ui.set_other(ModelRc::default());
                ui.set_status(format!("Could not read saved materials. {error}").into());
            }
        }
        if let Some(course) = view.courses.iter().find(|course| course.id == id) {
            ui.set_heading(course.name.clone().into());
        }
        ui.set_summary(file_count(view.files.len() as i64).into());
    } else {
        match library.courses() {
            Ok(courses) => view.courses = courses,
            Err(error) => ui.set_status(format!("Could not read modules. {error}").into()),
        }
        ui.set_summary(
            format!(
                "{} {}",
                view.courses.len(),
                if view.courses.len() == 1 {
                    "module"
                } else {
                    "modules"
                }
            )
            .into(),
        );
        let counts = library.file_counts().unwrap_or_default();
        let rows = view
            .courses
            .iter()
            .map(|course| LibraryRow {
                title: course.name.clone().into(),
                detail: file_count(*counts.get(&course.id).unwrap_or(&0)).into(),
                ..Default::default()
            })
            .collect::<Vec<_>>();
        ui.set_rows(ModelRc::new(VecModel::from(rows)));
    }
    if let Ok(Some(timestamp)) = library.last_sync() {
        if let Some(time) = chrono::DateTime::from_timestamp(timestamp, 0) {
            ui.set_last_sync(
                format!(
                    "Last successful sync · {}",
                    time.with_timezone(&chrono::Local).format("%d %b %Y, %H:%M")
                )
                .into(),
            );
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MaterialCategory {
    Lecture,
    Tutorial,
    Other,
}

fn material_category(title: &str, filename: &str, section: &str) -> MaterialCategory {
    for label in [title, filename, section] {
        let mut lecture = false;
        let mut tutorial = false;
        for word in label.split(|character: char| !character.is_alphanumeric()) {
            let word = word.to_ascii_lowercase();
            let name = word.trim_end_matches(|character: char| character.is_ascii_digit());
            match name {
                "lecture" | "lectures" | "lec" => lecture = true,
                "tutorial" | "tutorials" | "tut" => tutorial = true,
                "l" if name.len() < word.len() => lecture = true,
                "t" if name.len() < word.len() => tutorial = true,
                _ => (),
            }
        }
        match (lecture, tutorial) {
            (true, false) => return MaterialCategory::Lecture,
            (false, true) => return MaterialCategory::Tutorial,
            _ => (),
        }
    }
    MaterialCategory::Other
}

fn material_columns(files: &[SavedFile]) -> [Vec<LibraryRow>; 3] {
    let mut columns: [Vec<LibraryRow>; 3] = Default::default();
    let mut previous_section = None;
    for (index, file) in files.iter().enumerate() {
        let column = match material_category(&file.title, &file.filename, &file.section) {
            MaterialCategory::Lecture => 0,
            MaterialCategory::Tutorial => 1,
            MaterialCategory::Other => 2,
        };
        let section = file.section.as_str();
        let mut group = "";
        if column == 2 {
            if previous_section != Some(section) {
                group = section;
            }
            previous_section = Some(section);
        }
        let kind = std::path::Path::new(&file.filename)
            .extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or("FILE");
        columns[column].push(LibraryRow {
            title: file.title.clone().into(),
            detail: file.filename.clone().into(),
            kind: kind.to_ascii_uppercase().into(),
            group: group.into(),
            file_index: index as i32,
        });
    }
    columns
}

#[cfg(test)]
mod material_view_tests {
    use super::*;

    #[test]
    fn lecture_and_tutorial_rows_are_flat_and_keep_original_file_indices() {
        let files = [
            ("L1", "slides.pdf", "Week 1"),
            ("Exercises", "TUT1.pdf", "Week 2"),
            ("Lab instructions", "lab.pdf", "Week 1"),
            ("Lecture notes", "notes.pdf", "Week 3"),
            ("Practice", "sheet.pdf", "Tutorials"),
        ]
        .map(|(title, filename, section)| SavedFile {
            id: filename.into(),
            title: title.into(),
            filename: filename.into(),
            section: section.into(),
            path: PathBuf::from(filename),
        });
        let [lectures, tutorials, other] = material_columns(&files);
        assert_eq!((lectures[0].file_index, lectures[1].file_index), (0, 3));
        assert_eq!((tutorials[0].file_index, tutorials[1].file_index), (1, 4));
        assert_eq!(other[0].file_index, 2);
        assert!(lectures
            .iter()
            .chain(&tutorials)
            .all(|row| row.group.is_empty()));
        assert_eq!(other[0].group, "Week 1");
    }
}

fn file_count(count: i64) -> String {
    format!(
        "{count} {} saved",
        if count == 1 { "file" } else { "files" }
    )
}

fn open_saved(
    root: &std::path::Path,
    course: i64,
    file: &SavedFile,
    open: impl Fn(&std::path::Path) -> std::io::Result<()>,
) -> Result<()> {
    if file.path.is_file() {
        match open(&file.path) {
            Ok(()) => return Ok(()),
            Err(error) if file.path.is_file() => {
                return Err(error).context("Could not open this file. Check your default viewer.");
            }
            Err(_) => (),
        }
    }
    // A sync may have published a new copy since this row was displayed.
    let path = Library::cached_path(root, course, &file.id)?
        .filter(|path| path.is_file())
        .context("This file is missing. Sync again to download it.")?;
    open(&path).context("Could not open this file. Check your default viewer.")
}

struct Report {
    message: String,
    connected: bool,
    busy: bool,
    refresh_library: bool,
    open_modules: bool,
}

fn worker(
    root: PathBuf,
    incoming: mpsc::Receiver<Request>,
    outgoing: mpsc::Sender<Report>,
    signal: slint::Weak<AppWindow>,
) {
    let report = |message: String, connected, busy, refresh_library, open_modules| {
        let _ = outgoing.send(Report {
            message,
            connected,
            busy,
            refresh_library,
            open_modules,
        });
        let _ = signal.upgrade_in_event_loop(|ui| ui.invoke_read_sync_status());
    };
    let startup = || -> Result<_> {
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(root.join("sync.lock"))?;
        lock.try_lock()
            .context("Another Elearnin’ window is already handling sync")?;
        let library = Library::open(&root)?;
        let portal = Portal::new()?;
        let key = keyring::Entry::new("Elearnin BUE Learn1", "saved-sign-in")
            .context("Could not open your system credential store")?;
        Ok((lock, library, portal, key))
    };
    let (_lock, mut library, mut portal, key) = match startup() {
        Ok(state) => state,
        Err(error) => {
            report(error.to_string(), false, false, false, false);
            return;
        }
    };
    let mut connected = false;
    let mut retry_automatically = true;
    loop {
        let request = match incoming.recv_timeout(SYNC_INTERVAL) {
            Ok(request) => request,
            Err(mpsc::RecvTimeoutError::Timeout) if connected && retry_automatically => {
                Request::Refresh
            }
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => return,
        };
        let mut open_modules = false;
        let result = (|| -> Result<String> {
            match request {
                Request::Connect(credentials) => {
                    let saved =
                        serde_json::to_string(&(&credentials.username, &credentials.password))?;
                    key.set_password(&saved).context("Could not save credentials. Check that your system keyring is available and unlocked.")?;
                    connected = true;
                    portal = Portal::new()?;
                    open_modules = true;
                }
                Request::Disconnect => {
                    match key.delete_credential() {
                        Ok(()) | Err(keyring::Error::NoEntry) => (),
                        Err(error) => return Err(error).context("Could not remove saved sign-in"),
                    }
                    connected = false;
                    portal = Portal::new()?;
                    return Ok(
                        "Sign-in removed. Your downloaded materials are still available.".into(),
                    );
                }
                Request::Refresh => (),
            }
            let secret =
                match key.get_password() {
                    Ok(value) => value,
                    Err(keyring::Error::NoEntry) => {
                        connected = false;
                        return Ok(
                            "Connect your BUE account in Settings to start automatic sync.".into(),
                        );
                    }
                    Err(error) => return Err(error).context(
                        "Could not read saved sign-in. Check that your system keyring is unlocked.",
                    ),
                };
            connected = true;
            let (username, password) = serde_json::from_str::<(String, String)>(&secret).context(
                "Could not read saved sign-in. Save your credentials again in Settings.",
            )?;
            let credentials = Credentials { username, password };
            report("Checking BUE for modules…".into(), true, true, false, false);
            let mut sync = |portal: &mut Portal| {
                sync_library(&mut library, portal, &credentials, |message| {
                    report(message, true, true, true, false)
                })
            };
            let count = match sync(&mut portal) {
                Err(error) if error.is::<Expired>() => {
                    portal = Portal::new()?;
                    sync(&mut portal)?
                }
                result => result?,
            };
            Ok(format!(
                "Synced {count} {} from BUE.",
                if count == 1 { "module" } else { "modules" }
            ))
        })();
        let succeeded = result.is_ok();
        retry_automatically = !result
            .as_ref()
            .is_err_and(|error| error.is::<bue::SignInRejected>());
        let message = result.unwrap_or_else(|error| error.to_string());
        report(message, connected, false, true, open_modules && succeeded);
    }
}

fn sync_library(
    library: &mut Library,
    portal: &mut Portal,
    credentials: &Credentials,
    progress: impl Fn(String),
) -> Result<usize> {
    let courses = portal.courses(credentials)?;
    for course in &courses {
        library.save_course(course)?;
    }
    let mut skipped = 0;
    let mut last_error = None;
    for (index, course) in courses.iter().enumerate() {
        progress(format!(
            "Syncing {} ({}/{})…",
            course.name,
            index + 1,
            courses.len()
        ));
        let result = portal.materials(course.id).and_then(|materials| {
            let mut last_failure = materials
                .issues
                .last()
                .map(|issue| anyhow::anyhow!("{issue}"));
            for item in materials.files {
                if let Err(error) =
                    library.save_material(course.id, &item, |output| portal.download(&item, output))
                {
                    if error.is::<Expired>() {
                        return Err(error);
                    }
                    last_failure =
                        Some(error.context(format!("{} · {}", item.section, item.title)));
                }
            }
            last_failure.map_or(Ok(()), Err)
        });
        if let Err(error) = result {
            if error.is::<Expired>() {
                return Err(error);
            }
            skipped += 1;
            last_error = Some(error.context(course.name.clone()));
        }
    }
    if let Some(error) = last_error {
        anyhow::bail!(
            "Sync incomplete for {skipped} of {} modules. {error:#}",
            courses.len()
        );
    }
    library.mark_synced()?;
    Ok(courses.len())
}

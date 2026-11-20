use super::App;
use eframe::egui;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    io::{ErrorKind, Write},
    path::{Path, PathBuf},
    sync::mpsc,
    time::{Duration, Instant, SystemTime},
};

const SCAN_INTERVAL: Duration = Duration::from_millis(750);

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(super) enum Version {
    Text([u8; 32], usize),
    Missing,
    Unreadable,
}
pub(super) fn version(text: &str) -> Version {
    Version::Text(Sha256::digest(text.as_bytes()).into(), text.len())
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Stamp {
    length: u64,
    modified: Option<SystemTime>,
}
fn stamp(path: &Path) -> Option<Stamp> {
    let meta = fs::metadata(path).ok()?;
    Some(Stamp {
        length: meta.len(),
        modified: meta.modified().ok(),
    })
}

pub(super) struct Snapshot {
    pub text: Result<Option<String>, String>,
    pub version: Version,
    stamp: Option<Stamp>,
}
impl Snapshot {
    pub fn read(path: &Path) -> Self {
        let stamp = stamp(path);
        let text = if fs::metadata(path).is_err_and(|e| e.kind() == ErrorKind::NotFound) {
            Ok(None)
        } else {
            path.to_str()
                .ok_or_else(|| "Invalid file path".to_owned())
                .and_then(|path| crate::cli::input(path).map(Some).map_err(|e| e.to_string()))
        };
        let version = match &text {
            Ok(Some(text)) => version(text),
            Ok(None) => Version::Missing,
            Err(_) => Version::Unreadable,
        };
        Self {
            text,
            version,
            stamp,
        }
    }
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct File {
    pub path: PathBuf,
    pub saved: Version,
    pub observed: Version,
    #[serde(skip)]
    stamp: Option<Stamp>,
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Choice {
    Reload,
    Overwrite,
}
pub(super) struct Overwrite {
    name: String,
    path: PathBuf,
    observed: Version,
    focus_yes: bool,
}
impl File {
    pub fn new(path: PathBuf, text: &str) -> Self {
        let stamp = stamp(&path);
        let saved = version(text);
        Self {
            path,
            saved,
            observed: saved,
            stamp,
        }
    }
    pub fn observe(&mut self, snapshot: &Snapshot) {
        self.stamp = snapshot.stamp;
        self.observed = snapshot.version;
    }
    pub fn out_of_sync(&self) -> bool {
        self.saved != self.observed
    }
    pub fn matches_saved(&self, text: &str) -> bool {
        matches!(self.saved, Version::Text(_, len) if len == text.len())
            && self.saved == version(text)
    }
}

type WatchList = Vec<(String, PathBuf, Option<Stamp>)>;
type Changes = Vec<(String, Snapshot)>;
pub(super) struct Watcher {
    sender: mpsc::Sender<(u64, bool, WatchList)>,
    receiver: mpsc::Receiver<(u64, Changes)>,
    pub files: BTreeMap<String, File>,
    epoch: u64,
    pending: bool,
    focused: bool,
    next_scan: Instant,
}
impl Watcher {
    pub fn new(ctx: egui::Context) -> Self {
        let (sender, requests) = mpsc::channel::<(u64, bool, WatchList)>();
        let (replies, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            while let Ok((epoch, refocused, files)) = requests.recv() {
                let changes = files
                    .into_iter()
                    .filter(|(_, path, previous)| refocused || stamp(path) != *previous)
                    .map(|(name, path, _)| (name, Snapshot::read(&path)))
                    .collect();
                if replies.send((epoch, changes)).is_err() {
                    break;
                }
                ctx.request_repaint();
            }
        });
        Self {
            sender,
            receiver,
            files: BTreeMap::new(),
            epoch: 0,
            pending: false,
            focused: false,
            next_scan: Instant::now(),
        }
    }
    pub fn invalidate(&mut self) {
        self.epoch += 1;
        self.next_scan = Instant::now();
    }
    pub fn tick(&mut self, focused: bool, ctx: &egui::Context, now: Instant) -> Changes {
        let refocused = focused && !self.focused;
        self.focused = focused;
        if !focused || self.files.is_empty() {
            return Vec::new();
        }
        let changes = self.receiver.try_recv().ok().and_then(|(epoch, changes)| {
            self.pending = false;
            (epoch == self.epoch).then_some(changes)
        });
        if refocused {
            self.next_scan = now;
        }
        if !self.pending && now >= self.next_scan {
            let files = self
                .files
                .iter()
                .map(|(name, file)| (name.clone(), file.path.clone(), file.stamp))
                .collect();
            self.pending = self.sender.send((self.epoch, refocused, files)).is_ok();
            self.next_scan = now + SCAN_INTERVAL;
        }
        ctx.request_repaint_after(
            self.next_scan
                .saturating_duration_since(now)
                .max(SCAN_INTERVAL / 4),
        );
        changes.unwrap_or_default()
    }
}

impl App {
    pub(super) fn reset_disk_choices(&mut self) {
        self.source_rows.clear();
        self.conflict_choice = None;
        self.overwrite = None;
        self.last_f5 = None;
        self.find.results = None;
        self.find.reveal = false;
        self.disk.invalidate();
    }

    pub(super) fn update_title(&mut self, ctx: &egui::Context) {
        let title = match self.disk.files.get(&self.selected) {
            Some(file) => {
                let name = file
                    .path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or(&self.selected);
                format!("{name} | {} | CNC Viewer", file.path.display())
            }
            None => format!("{} | CNC Viewer", self.selected),
        };
        if title != self.title {
            self.title = title.clone();
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title));
        }
    }

    pub(super) fn watch_disk(&mut self, ctx: &egui::Context, focused: bool, now: Instant) {
        for (name, snapshot) in self.disk.tick(focused, ctx, now) {
            self.observe_disk(&name, snapshot, ctx);
        }
    }

    fn observe_disk(&mut self, name: &str, snapshot: Snapshot, ctx: &egui::Context) {
        let Some(file) = self.disk.files.get_mut(name) else {
            return;
        };
        if file.observed != snapshot.version
            && self
                .conflict_choice
                .as_ref()
                .is_some_and(|(selected, _)| selected == name)
        {
            self.conflict_choice = None;
        }
        file.observe(&snapshot);
        if let Ok(Some(text)) = snapshot.text {
            if self.documents.get(name) == Some(&text) {
                file.saved = snapshot.version;
                self.dirty.remove(name);
            } else if !self.dirty.contains(name) {
                self.reload_from_snapshot(name, text, snapshot.version, ctx);
            }
        }
    }

    fn reload_from_snapshot(
        &mut self,
        name: &str,
        text: String,
        version: Version,
        ctx: &egui::Context,
    ) {
        self.documents.insert(name.to_owned(), text);
        self.dirty.remove(name);
        if self.selected == name {
            self.find.results = None;
            self.find.reveal = false;
        }
        if let Some(file) = self.disk.files.get_mut(name) {
            file.saved = version;
        }
        self.highlight_cache.remove(name);
        self.source_rows.remove(name);
        ctx.data_mut(|data| {
            data.remove::<egui::text_edit::TextEditState>(self.editor_id_for(name));
        });
        self.conflict_choice = None;
        self.disk.invalidate();
        self.schedule_refresh(ctx, Instant::now());
    }

    fn reload_disk(&mut self, name: &str, ctx: &egui::Context) {
        let Some(file) = self.disk.files.get_mut(name) else {
            return;
        };
        let snapshot = Snapshot::read(&file.path);
        file.observe(&snapshot);
        match snapshot.text {
            Ok(Some(text)) => self.reload_from_snapshot(name, text, snapshot.version, ctx),
            Ok(None) => self.status = "File is missing on disk".into(),
            Err(err) => self.status = err,
        }
    }

    pub(super) fn refresh_key(&mut self, ctx: &egui::Context, now: Instant) {
        let name = self.selected.clone();
        if !self.disk.files.contains_key(&name) {
            self.replot();
            return;
        }
        let repeat = self.last_f5.as_ref().is_some_and(|(last, when)| {
            *last == name && now.saturating_duration_since(*when) <= Duration::from_millis(900)
        });
        if repeat || !self.dirty.contains(&name) {
            self.last_f5 = None;
            self.reload_disk(&name, ctx);
        } else {
            self.last_f5 = Some((name.clone(), now));
            if let Some(file) = self.disk.files.get(&name) {
                let snapshot = Snapshot::read(&file.path);
                self.observe_disk(&name, snapshot, ctx);
            }
            self.status = "Press F5 again to reload and discard edits".into();
        }
    }

    pub(super) fn save(&mut self, as_new: bool) {
        let name = self.selected.clone();
        self.save_document(&name, as_new);
    }

    pub(super) fn save_document(&mut self, name: &str, as_new: bool) -> bool {
        let path = if as_new || !self.disk.files.contains_key(name) {
            rfd::FileDialog::new().set_file_name(name).save_file()
        } else {
            self.disk.files.get(name).map(|file| file.path.clone())
        };
        let Some(path) = path else { return false };
        let path = std::path::absolute(&path).unwrap_or(path);
        let snapshot = Snapshot::read(&path);
        let tracked = self
            .disk
            .files
            .iter()
            .find(|(_, file)| {
                file.path == path
                    || fs::canonicalize(&path).ok().is_some_and(|path| {
                        fs::canonicalize(&file.path).is_ok_and(|file| file == path)
                    })
            })
            .map(|(name, _)| name.clone());
        if let Some(tracked) = tracked {
            if tracked != name {
                self.status = "That path belongs to another open source".into();
                return false;
            }
            let file = self.disk.files.get_mut(name).expect("tracked source");
            file.observe(&snapshot);
            if file.out_of_sync()
                && snapshot.text.as_ref().ok().and_then(Option::as_ref) != self.documents.get(name)
            {
                self.overwrite = Some(Overwrite {
                    name: name.to_owned(),
                    path,
                    observed: snapshot.version,
                    focus_yes: true,
                });
                return false;
            }
        }
        self.write_document(name, path, snapshot.version)
    }

    fn write_document(&mut self, name: &str, path: PathBuf, expected: Version) -> bool {
        let Some(text) = self.documents.get(name) else {
            return false;
        };
        let result = crate::cli::write_artifact(&path, |mut file| {
            file.write_all(text.as_bytes())?;
            if Snapshot::read(&path).version != expected {
                return Err("File changed again. Review it before overwriting.".into());
            }
            Ok(())
        });
        match result {
            Ok(()) => {
                self.status = format!("Saved {}", path.display());
                self.dirty.remove(name);
                self.disk
                    .files
                    .insert(name.to_owned(), File::new(path, text));
                self.disk.invalidate();
                self.conflict_choice = None;
                true
            }
            Err(err) => {
                self.status = err.to_string();
                self.disk.invalidate();
                false
            }
        }
    }

    pub(super) fn apply_disk_choice(&mut self, name: &str, choice: Choice, ctx: &egui::Context) {
        match choice {
            Choice::Reload => self.reload_disk(name, ctx),
            Choice::Overwrite => {
                if let Some(file) = self.disk.files.get(name) {
                    self.write_document(name, file.path.clone(), file.observed);
                }
            }
        }
    }

    pub(super) fn confirm_overwrite(&mut self) -> bool {
        let Some(Overwrite {
            name,
            path,
            observed,
            ..
        }) = self.overwrite.take()
        else {
            return false;
        };
        self.write_document(&name, path, observed)
    }

    pub(super) fn overwrite_modal(&mut self, ctx: &egui::Context) {
        let Some(overwrite) = &mut self.overwrite else {
            return;
        };
        let mut yes = false;
        let mut no = false;
        let file = overwrite.path.display().to_string();
        let focus_yes = std::mem::take(&mut overwrite.focus_yes);
        let response = egui::Modal::new(self.local_id("overwrite-disk")).show(ctx, |ui| {
            ui.heading("Overwrite file?");
            ui.label(file);
            ui.horizontal(|ui| {
                let button = ui.button("Yes");
                if focus_yes {
                    button.request_focus();
                }
                yes = button.clicked();
                no = ui.button("No").clicked();
            });
        });
        if yes {
            self.confirm_overwrite();
        } else if no || response.should_close() {
            self.overwrite = None;
        }
    }
}

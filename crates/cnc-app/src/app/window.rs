use super::{
    history::History,
    project::{self, Project},
    session::{self, OpenTab, Session},
    tabs, App, DocumentAction,
};
use eframe::egui::{
    self, pos2, vec2, Color32, Key, Modifiers, PointerButton, Pos2, Rect, Sense, Stroke,
};
use std::{
    collections::BTreeSet,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::mpsc,
    time::{Duration, Instant},
};

type Tab = (usize, usize);
type StartupError = Box<dyn std::error::Error + Send + Sync>;

struct Slot {
    parked: Option<Project>,
    published: Option<OpenTab>,
}

struct Pane {
    viewer: App,
    tabs: Vec<Slot>,
    active: usize,
    reveal: bool,
}

impl Pane {
    fn new(viewer: App) -> Self {
        Self {
            viewer,
            tabs: Vec::new(),
            active: 0,
            reveal: false,
        }
    }

    fn active_path(&self) -> Option<&Path> {
        self.viewer
            .disk
            .files
            .get(&self.viewer.main)
            .map(|file| file.path.as_path())
    }

    fn labels(&self) -> Vec<tabs::Label> {
        self.tabs
            .iter()
            .map(|slot| {
                if let Some(project) = &slot.parked {
                    tabs::Label {
                        text: project.label().into(),
                        path: project.path().map(Path::to_owned),
                        dirty: !project.dirty.is_empty(),
                    }
                } else {
                    tabs::Label {
                        text: project::file_label(&self.viewer.main, self.active_path()).into(),
                        path: self.active_path().map(Path::to_owned),
                        dirty: !self.viewer.dirty.is_empty(),
                    }
                }
            })
            .collect()
    }

    fn publish(&mut self, session: &Session) -> io::Result<()> {
        for slot in &mut self.tabs {
            let record = if let Some(parked) = &slot.parked {
                parked.record()
            } else {
                project::tab_record(
                    &self.viewer.project_id,
                    &self.viewer.main,
                    &self.viewer.disk.files,
                    &self.viewer.program_setup,
                    self.viewer.opened_at,
                    self.viewer.edited_at,
                )
            };
            if record == slot.published {
                continue;
            }
            if let Some(record) = &record {
                session.record(record.clone())?;
            } else if let Some(old) = &slot.published {
                session.forget(&BTreeSet::from([old.id.as_str()]))?;
            }
            slot.published = record;
        }
        Ok(())
    }

    fn add(&mut self, project: Project, published: Option<OpenTab>) {
        if !self.tabs.is_empty() {
            self.tabs[self.active].parked = Some(self.viewer.take_project());
        }
        self.active = self.tabs.len();
        self.tabs.push(Slot {
            parked: None,
            published,
        });
        self.viewer.install_project(project);
        self.reveal = true;
    }

    fn select(&mut self, index: usize) {
        if index == self.active {
            return;
        }
        if let Some(project) = self.tabs[index].parked.take() {
            self.tabs[self.active].parked = Some(self.viewer.take_project());
            self.active = index;
            self.viewer.install_project(project);
            self.reveal = true;
        }
    }

    fn remove(&mut self, index: usize) -> (Project, Option<OpenTab>) {
        let slot = self.tabs.remove(index);
        if let Some(project) = slot.parked {
            self.active -= usize::from(index < self.active);
            return (project, slot.published);
        }
        let previous = self.viewer.take_project();
        if !self.tabs.is_empty() {
            self.active = index.min(self.tabs.len() - 1);
            self.viewer.install_project(
                self.tabs[self.active]
                    .parked
                    .take()
                    .expect("Parked replacement"),
            );
            self.reveal = true;
        }
        (previous, slot.published)
    }

    fn path(&self, index: usize) -> Option<&Path> {
        match &self.tabs[index].parked {
            Some(project) => project.path(),
            None => self.active_path(),
        }
    }

    fn dirty(&self, index: usize) -> bool {
        match &self.tabs[index].parked {
            Some(project) => !project.dirty.is_empty(),
            None => !self.viewer.dirty.is_empty(),
        }
    }

    fn tab_id(&self, index: usize) -> &str {
        self.tabs[index]
            .parked
            .as_ref()
            .map_or(&self.viewer.project_id, |p| &p.id)
    }

    fn touch_open(&mut self, index: usize) {
        self.select(index);
        self.viewer.opened_at = session::now();
    }

    fn reorder(&mut self, from: usize, to: usize) {
        if from == to {
            return;
        }
        let slot = self.tabs.remove(from);
        self.tabs.insert(to, slot);
        self.active = self
            .tabs
            .iter()
            .position(|slot| slot.parked.is_none())
            .expect("Active tab");
    }
}

#[derive(Clone, Copy)]
enum PendingClose {
    Tab(Tab),
    Window,
}

struct Transfer {
    tab: Tab,
    result: mpsc::Receiver<Result<(), String>>,
}

#[derive(Default)]
struct DropTargets {
    bar: Option<Rect>,
    tabs: Vec<(usize, Rect)>,
}

pub struct Window {
    panes: Vec<Pane>,
    focused: usize,
    split: f32,
    session: Option<Session>,
    history: History,
    next_publish: Instant,
    serial: u64,
    prefix: String,
    pending: Option<PendingClose>,
    allow_close: bool,
    drag: Option<Tab>,
    transfer: Option<Transfer>,
    acknowledge_transfer: bool,
    closed_panes: Vec<Pane>,
}

impl Window {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        file: Option<PathBuf>,
        receiving: bool,
    ) -> Result<Self, StartupError> {
        let transferred: Option<Project> = receiving
            .then(|| serde_json::from_reader(io::stdin().lock()))
            .transpose()?;
        let viewer = App::new(cc)?;
        let mut session_error = None;
        let (session, candidates) = if viewer.capture_path.is_none() {
            let settings = super::settings_path();
            match Session::start(
                settings.parent().expect("Settings directory").to_owned(),
                file.is_none() && !receiving,
            ) {
                Ok((session, candidates)) => (Some(session), candidates),
                Err(error) => {
                    session_error = Some(format!("Could not open tab history: {error}"));
                    (None, Vec::new())
                }
            }
        } else {
            (None, Vec::new())
        };
        let mut window = Self::empty(viewer, session);
        window.acknowledge_transfer = receiving;
        if let Some(project) = transferred {
            window.panes[0].add(project, None);
        } else if let Some(file) = file {
            let recent_id = window.new_id();
            match Project::open(recent_id, &file) {
                Ok(project) => window.panes[0].add(project, None),
                Err(error) => {
                    session_error = Some(format!("Could not open {}: {error}", file.display()))
                }
            }
        } else {
            let mut restored = restore_projects(&candidates);
            let discarded = candidates
                .iter()
                .filter(|tab| !restored.iter().any(|(_, saved)| tab.id == saved.id))
                .map(|tab| tab.id.as_str())
                .collect::<BTreeSet<_>>();
            if let Some(session) = &window.session {
                if !discarded.is_empty() {
                    session.forget(&discarded)?;
                }
            }
            // Only the selected restored project needs an interpreter job at startup.
            if !restored.is_empty() {
                let (project, published) = restored.remove(0);
                window.panes[0]
                    .tabs
                    .extend(restored.into_iter().rev().map(|(project, record)| Slot {
                        parked: Some(project),
                        published: Some(record),
                    }));
                let pane = &mut window.panes[0];
                pane.active = pane.tabs.len();
                pane.tabs.push(Slot {
                    parked: None,
                    published: Some(published),
                });
                pane.viewer.install_project(project);
                pane.reveal = true;
            }
        }
        if window.panes[0].tabs.is_empty() {
            window.add_starter(0);
        }
        window.publish();
        if let Some(message) = session_error {
            window.panes[0].viewer.status = message;
        }
        Ok(window)
    }

    fn empty(viewer: App, session: Option<Session>) -> Self {
        Self {
            panes: vec![Pane::new(viewer)],
            focused: 0,
            split: 0.5,
            history: History::new(),
            session,
            next_publish: Instant::now(),
            serial: 0,
            prefix: format!("{}-{}", std::process::id(), session::now()),
            pending: None,
            allow_close: false,
            drag: None,
            transfer: None,
            acknowledge_transfer: false,
            closed_panes: Vec::new(),
        }
    }

    fn new_id(&mut self) -> String {
        self.serial += 1;
        format!("{}-{}", self.prefix, self.serial)
    }

    fn add_starter(&mut self, pane: usize) {
        let project = Project::new(self.new_id(), project::UNTITLED, project::STARTER);
        self.panes[pane].add(project, None);
    }

    fn publish(&mut self) {
        self.next_publish = Instant::now() + Duration::from_secs(1);
        if let Some(session) = &self.session {
            for pane in &mut self.panes {
                if let Err(error) = pane.publish(session) {
                    pane.viewer.status = format!("Could not save tab history: {error}");
                }
            }
        }
    }

    fn set_focus(&mut self, pane: usize) {
        if self.focused != pane {
            self.focused = pane;
            self.panes[pane].viewer.title.clear();
        }
    }

    fn insert(&mut self, pane: usize, project: Project) {
        let viewer = &self.panes[pane].viewer;
        if self.panes[pane].tabs.len() == 1
            && viewer.main == project::UNTITLED
            && viewer
                .documents
                .get(project::UNTITLED)
                .is_some_and(|text| text == project::STARTER)
            && self.panes[pane].active_path().is_none()
            && viewer.dirty.is_empty()
        {
            self.panes[pane].remove(0);
        }
        self.panes[pane].add(project, None);
        self.set_focus(pane);
    }

    fn open(&mut self, pane: usize, path: &Path) {
        let canonical = session::canonical(path);
        for index in 0..self.panes.len() {
            if let Some(tab) = (0..self.panes[index].tabs.len()).find(|&tab| {
                self.panes[index]
                    .path(tab)
                    .is_some_and(|p| session::canonical(p) == canonical)
            }) {
                self.panes[index].touch_open(tab);
                self.set_focus(index);
                return;
            }
        }
        let id = self.new_id();
        match Project::open(id, path) {
            Ok(project) => self.insert(pane, project),
            Err(error) => {
                self.panes[pane].viewer.status =
                    format!("Could not open {}: {error}", path.display())
            }
        }
    }

    fn document_action(&mut self, pane: usize, action: DocumentAction) {
        match action {
            DocumentAction::Open(paths) => {
                for path in paths {
                    self.open(pane, &path);
                }
            }
            DocumentAction::New => {
                let id = self.new_id();
                let suffix = self.serial;
                self.insert(pane, Project::new(id, &format!("New {suffix}.nc"), ""));
            }
            DocumentAction::Sample => {
                if let Some(index) = self.panes[pane]
                    .labels()
                    .iter()
                    .position(|label| label.text == project::SAMPLE && label.path.is_none())
                {
                    self.panes[pane].select(index);
                } else {
                    let id = self.new_id();
                    self.insert(
                        pane,
                        Project::new(
                            id,
                            project::SAMPLE,
                            include_str!("../../../../examples/CNCViewerDemo.nc"),
                        ),
                    );
                }
            }
        }
    }

    fn normalize_panes(&mut self, ctx: &egui::Context) {
        if self.panes.len() == 2 && self.panes.iter().any(|pane| pane.tabs.is_empty()) {
            let index = usize::from(!self.panes[0].tabs.is_empty());
            self.closed_panes.push(self.panes.remove(index));
            self.focused = 0;
            self.panes[0].viewer.title.clear();
            ctx.send_viewport_cmd(egui::ViewportCommand::MinInnerSize(vec2(640., 400.)));
        }
        if self.panes[0].tabs.is_empty() {
            self.add_starter(0);
        }
    }

    fn split(&mut self, from: Tab, ctx: &egui::Context) {
        let other = 1 - from.0;
        if self.panes.len() == 1 {
            let gpu = self.panes[0].viewer.gpu.clone();
            self.panes.push(Pane::new(App::with_gpu(ctx, gpu)));
            ctx.send_viewport_cmd(egui::ViewportCommand::MinInnerSize(vec2(1000., 430.)));
            let size = ctx.input(|input| input.content_rect().size());
            if size.x < 1000. || size.y < 430. {
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(
                    size.max(vec2(1000., 430.)),
                ));
            }
            self.split = 0.5;
            let (project, saved) = self.panes[from.0].remove(from.1);
            if self.panes[0].tabs.is_empty() {
                self.add_starter(0);
            }
            self.panes[other].add(project, saved);
        } else {
            let (project, saved) = self.panes[from.0].remove(from.1);
            self.panes[other].add(project, saved);
        }
        self.set_focus(other);
        self.normalize_panes(ctx);
    }

    fn start_transfer(&mut self, tab: Tab, ctx: &egui::Context) {
        let pane = &self.panes[tab.0];
        let snapshot = pane.tabs[tab.1]
            .parked
            .clone()
            .unwrap_or_else(|| pane.viewer.snapshot_project());
        let (tx, rx) = mpsc::channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let result = send_to_window(snapshot).map_err(|error| error.to_string());
            let _ = tx.send(result);
            ctx.request_repaint();
        });
        self.transfer = Some(Transfer { tab, result: rx });
    }

    fn receive_transfer(&mut self, ctx: &egui::Context) {
        let Some(transfer) = &self.transfer else {
            return;
        };
        let outcome = match transfer.result.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => Err("New window stopped responding".into()),
        };
        let (pane, index) = transfer.tab;
        self.transfer = None;
        match outcome {
            Ok(()) => {
                let (moved, _) = self.panes[pane].remove(index);
                forget_project_history(ctx, &moved);
                self.normalize_panes(ctx);
            }
            Err(error) => self.panes[pane].viewer.status = format!("Could not move tab: {error}"),
        }
    }

    fn close_tab(&mut self, tab: Tab, ctx: &egui::Context) {
        let pane = &mut self.panes[tab.0];
        if let Some(session) = &self.session {
            if let Err(error) = session.forget(&BTreeSet::from([pane.tab_id(tab.1)])) {
                pane.viewer.status = format!("Could not update tab history: {error}");
                return;
            }
        }
        let (removed, _) = pane.remove(tab.1);
        forget_project_history(ctx, &removed);
        self.normalize_panes(ctx);
    }

    fn dirty_tabs(&self, close: PendingClose) -> Vec<Tab> {
        match close {
            PendingClose::Tab(tab) => self.panes[tab.0]
                .dirty(tab.1)
                .then_some(tab)
                .into_iter()
                .collect(),
            PendingClose::Window => self
                .panes
                .iter()
                .enumerate()
                .flat_map(|(pane, tabs)| {
                    (0..tabs.tabs.len())
                        .filter(move |&tab| tabs.dirty(tab))
                        .map(move |tab| (pane, tab))
                })
                .collect(),
        }
    }

    fn complete_close(&mut self, close: PendingClose, ctx: &egui::Context) {
        self.pending = None;
        match close {
            PendingClose::Tab(tab) => self.close_tab(tab, ctx),
            PendingClose::Window => {
                self.publish();
                self.allow_close = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }

    fn request_close(&mut self, close: PendingClose, ctx: &egui::Context) {
        if self.dirty_tabs(close).is_empty() {
            self.complete_close(close, ctx);
        } else {
            self.pending = Some(close);
        }
    }

    fn close_prompt(&mut self, ctx: &egui::Context) {
        let Some(close) = self.pending else { return };
        if self
            .panes
            .iter()
            .any(|pane| pane.viewer.overwrite.is_some())
        {
            return;
        }
        let dirty = self.dirty_tabs(close);
        if dirty.is_empty() {
            self.complete_close(close, ctx);
            return;
        }
        let mut save = false;
        let mut discard = false;
        let mut cancel = false;
        let names = dirty
            .iter()
            .map(|&(pane, index)| self.panes[pane].labels()[index].text.clone())
            .collect::<Vec<_>>();
        egui::Modal::new(egui::Id::unique("close-tabs")).show(ctx, |ui| {
            ui.heading("Save changes?");
            ui.label(names.join(", "));
            ui.horizontal(|ui| {
                save = ui.button("Save").clicked();
                discard = ui.button("Discard").clicked();
                cancel = ui.button("Cancel").clicked();
            });
        });
        if save {
            let saved = dirty.into_iter().all(|(pane, index)| {
                self.set_focus(pane);
                self.panes[pane].select(index);
                let app = &mut self.panes[pane].viewer;
                let sources = app.dirty.iter().cloned().collect::<Vec<_>>();
                sources
                    .into_iter()
                    .all(|source| app.save_document(&source, false))
            });
            if saved {
                self.complete_close(close, ctx);
            }
        } else if discard {
            self.complete_close(close, ctx);
        } else if cancel || ctx.input(|input| input.key_pressed(Key::Escape)) {
            self.pending = None;
        }
    }

    fn tab_command(&mut self, pane: usize, command: tabs::Command, ctx: &egui::Context) {
        match command {
            tabs::Command::Select(index) => {
                self.panes[pane].select(index);
                self.set_focus(pane);
            }
            tabs::Command::New => self.document_action(pane, DocumentAction::New),
            tabs::Command::Sample => self.document_action(pane, DocumentAction::Sample),
            tabs::Command::Open(path) => self.open(pane, &path),
            tabs::Command::Close(index) => {
                self.request_close(PendingClose::Tab((pane, index)), ctx)
            }
            tabs::Command::Drag(index) => self.drag = Some((pane, index)),
            tabs::Command::Split(index) => self.split((pane, index), ctx),
            tabs::Command::Detach(index) => self.start_transfer((pane, index), ctx),
        }
    }
}

fn restore_projects(candidates: &[OpenTab]) -> Vec<(Project, OpenTab)> {
    let mut seen = BTreeSet::new();
    let mut restored = Vec::new();
    for tab in candidates {
        let path = session::canonical(&tab.path);
        if !seen.contains(&path) {
            if let Ok(project) = Project::restore(tab) {
                seen.insert(path);
                restored.push((project, tab.clone()));
                if restored.len() == session::RESTORE_LIMIT {
                    break;
                }
            }
        }
    }
    restored
}

fn forget_project_history(ctx: &egui::Context, project: &Project) {
    ctx.data_mut(|data| {
        for name in project.documents.keys() {
            data.remove::<egui::text_edit::TextEditState>(egui::Id::unique((
                &project.id,
                ("cnc-editor", name.as_str()),
            )));
        }
    });
}

fn send_to_window(project: Project) -> io::Result<()> {
    let mut child = Command::new(std::env::current_exe()?)
        .arg("--tab-transfer")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()?;
    let mut input = child.stdin.take().expect("Transfer input");
    let mut output = child.stdout.take().expect("Transfer acknowledgement");
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let result = (|| {
            serde_json::to_writer(&mut input, &project)?;
            drop(input);
            let mut byte = [0];
            output.read_exact(&mut byte)?;
            if byte != [b'\n'] {
                return Err(io::Error::other("New window rejected the tab"));
            }
            Ok(())
        })();
        let _ = tx.send(result);
    });
    let result = rx
        .recv_timeout(Duration::from_secs(30))
        .unwrap_or_else(|_| {
            Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "New window did not acknowledge the tab",
            ))
        });
    if result.is_err() {
        let _ = child.kill();
        let _ = child.wait();
    } else {
        std::thread::spawn(move || {
            let _ = child.wait();
        });
    }
    result
}

impl Window {
    fn draw(&mut self, ui: &mut egui::Ui) {
        self.closed_panes.clear();
        self.history.receive();
        let ctx = ui.ctx().clone();
        self.receive_transfer(&ctx);
        let full = ui.max_rect();
        let modal = self.pending.is_some()
            || self.transfer.is_some()
            || self
                .panes
                .iter()
                .any(|pane| pane.viewer.overwrite.is_some());
        if ui.input(|input| input.viewport().close_requested()) && !self.allow_close {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            if !modal {
                self.request_close(PendingClose::Window, &ctx);
            }
        }
        if !modal {
            let dropped = ui.input(|input| {
                input
                    .raw
                    .dropped_files
                    .iter()
                    .map(|file| file.path().to_path_buf())
                    .collect::<Vec<_>>()
            });
            if !dropped.is_empty() {
                self.publish();
                self.document_action(self.focused, DocumentAction::Open(dropped));
                self.publish();
            }
        }

        let mut rects = vec![full];
        if self.panes.len() == 2 {
            let min = (460. / full.width()).min(0.5);
            self.split = self.split.clamp(min, 1. - min);
            let middle = full.left() + full.width() * self.split;
            let divider = Rect::from_min_max(
                pos2(middle - 3., full.top()),
                pos2(middle + 3., full.bottom()),
            );
            let splitter = ui
                .interact(divider, ui.scope_id().with("viewer-split"), Sense::drag())
                .on_hover_cursor(egui::CursorIcon::ResizeHorizontal);
            if splitter.dragged() && !modal {
                self.split = (self.split
                    + ui.input(|input| input.pointer.delta().x) / full.width())
                .clamp(min, 1. - min);
            }
            ui.painter()
                .rect_filled(divider, 0., Color32::from_gray(210));
            rects[0].max.x = divider.left();
            rects.push(Rect::from_min_max(divider.right_top(), full.max));
        }
        if !modal && ui.input(|input| input.pointer.primary_pressed()) {
            if let Some(index) = ui
                .input(|input| input.pointer.interact_pos())
                .and_then(|p| rects.iter().position(|rect| rect.contains(p)))
            {
                self.set_focus(index);
            }
        }
        if self.drag.is_some()
            && ui.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Escape))
        {
            self.drag = None;
        }
        let command_close =
            !modal && ui.input_mut(|input| input.consume_key(Modifiers::COMMAND, Key::W));
        let mut commands = Vec::new();
        let mut targets = Vec::new();
        for (index, (pane, rect)) in self.panes.iter_mut().zip(&rects).enumerate() {
            let mut content = ui.new_child(
                egui::UiBuilder::new()
                    .id_salt(("viewer-pane", index))
                    .max_rect(*rect)
                    .layout(egui::Layout::top_down(egui::Align::Min)),
            );
            content.set_clip_rect(*rect);
            if self.transfer.is_some() {
                content.disable();
            }
            let labels = pane.labels();
            let mut strip = tabs::Strip {
                labels: &labels,
                selected: pane.active,
                history: &mut self.history,
                session: self.session.as_ref(),
                reveal: pane.reveal,
                commands: Vec::new(),
                rect: None,
                hits: Vec::new(),
            };
            pane.viewer
                .draw_ui(&mut content, !modal && index == self.focused, &mut strip);
            if strip.rect.is_some() {
                pane.reveal = false;
            }
            targets.push(DropTargets {
                bar: strip.rect,
                tabs: strip.hits,
            });
            for command in strip.commands {
                commands.push((index, command));
            }
        }
        if !modal {
            for index in 0..self.panes.len() {
                if let Some(action) = self.panes[index].viewer.action.take() {
                    self.publish();
                    self.document_action(index, action);
                    self.publish();
                }
            }
            if command_close {
                commands.push((
                    self.focused,
                    tabs::Command::Close(self.panes[self.focused].active),
                ));
            }
            for (pane, command) in commands {
                if !matches!(command, tabs::Command::Drag(_)) {
                    self.drag = None;
                    self.publish();
                }
                self.tab_command(pane, command, &ctx);
                ctx.request_repaint();
                if self.pending.is_some() || self.transfer.is_some() {
                    break;
                }
            }
            if let Some(from) = self.drag {
                let (position, left, released) = ui.input(|i| {
                    (
                        i.pointer.interact_pos(),
                        i.pointer.hover_pos().is_none(),
                        i.pointer.button_released(PointerButton::Primary),
                    )
                });
                let destination = drop_pane(
                    full,
                    &rects,
                    &targets,
                    from.0,
                    if left { None } else { position },
                );
                if released || left && outside_button_released() {
                    self.drag = None;
                    match destination {
                        DropOn::Window => self.start_transfer(from, &ctx),
                        DropOn::OtherPane => self.split(from, &ctx),
                        DropOn::SameTabBar(index) => self.panes[from.0].reorder(from.1, index),
                        DropOn::None => {}
                    }
                    self.publish();
                } else if left {
                    ctx.request_repaint_after(Duration::from_millis(16));
                } else if destination == DropOn::OtherPane {
                    let target = if rects.len() == 2 {
                        rects[1 - from.0]
                    } else {
                        Rect::from_min_max(pos2(full.center().x, full.top()), full.max)
                    };
                    ui.painter().rect_filled(
                        target,
                        0.,
                        Color32::from_rgba_unmultiplied(104, 182, 136, 35),
                    );
                    ui.painter().rect_stroke(
                        target.shrink(2.),
                        3.,
                        Stroke::new(2., Color32::from_rgb(104, 182, 136)),
                        egui::StrokeKind::Inside,
                    );
                }
            }
        }
        if self.transfer.is_some() {
            egui::Modal::new(egui::Id::unique("moving-tab")).show(&ctx, |ui| {
                ui.label("Opening a new window…");
            });
        } else {
            self.close_prompt(&ctx);
        }
        if self.next_publish <= Instant::now() {
            self.publish();
        }
        if self.session.is_some()
            && self.panes.iter().any(|pane| {
                pane.active_path().is_some()
                    && pane.tabs[pane.active]
                        .published
                        .as_ref()
                        .is_none_or(|record| record.edited != pane.viewer.edited_at)
            })
        {
            ctx.request_repaint_after(self.next_publish.saturating_duration_since(Instant::now()));
        }
        if std::mem::take(&mut self.acknowledge_transfer) {
            let _ = io::stdout().write_all(b"\n");
            let _ = io::stdout().flush();
        }
    }
}
impl eframe::App for Window {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.draw(ui);
    }
    fn on_exit(&mut self) {
        self.publish();
        self.panes[self.focused].viewer.save_settings();
    }
}

#[cfg(windows)]
fn outside_button_released() -> bool {
    #[link(name = "user32")]
    unsafe extern "system" {
        fn GetAsyncKeyState(key: i32) -> i16;
    }
    // Winit can report the button release after egui has dropped its pointer position.
    unsafe { GetAsyncKeyState(0x01) >= 0 }
}

#[cfg(not(windows))]
fn outside_button_released() -> bool {
    false
}

#[derive(Debug, PartialEq, Eq)]
enum DropOn {
    Window,
    OtherPane,
    SameTabBar(usize),
    None,
}

fn drop_pane(
    full: Rect,
    panes: &[Rect],
    targets: &[DropTargets],
    from: usize,
    pos: Option<Pos2>,
) -> DropOn {
    let Some(pos) = pos.filter(|&pos| full.contains(pos)) else {
        return DropOn::Window;
    };
    if targets[from].bar.is_some_and(|bar| bar.contains(pos)) {
        return targets[from]
            .tabs
            .iter()
            .find(|(_, rect)| rect.contains(pos))
            .or_else(|| targets[from].tabs.last())
            .map_or(DropOn::None, |(index, _)| DropOn::SameTabBar(*index));
    }
    if panes.get(1 - from).is_some_and(|pane| pane.contains(pos))
        || panes.len() == 1
            && pos.x >= full.center().x
            && pos.y >= full.top() + super::view::TOOLBAR_HEIGHT
    {
        DropOn::OtherPane
    } else {
        DropOn::None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::session::Recent;
    use std::fs;

    #[test]
    fn history_flattens_windows_skips_closed_and_missing_tabs_and_fills_seven() {
        let root = std::env::temp_dir().join(format!(
            "cnc-session-{}-{}",
            std::process::id(),
            session::now()
        ));
        let (first, empty) = Session::start(root.clone(), true).unwrap();
        assert!(empty.is_empty());
        for i in 0..=10 {
            let path = root.join(format!("part-{i}.nc"));
            if i != 10 {
                fs::write(&path, "G0 X1\nM30\n").unwrap();
            }
            first
                .record(OpenTab {
                    id: i.to_string(),
                    path,
                    opened: i,
                    edited: if i == 0 { 100 } else { 0 },
                    libraries: Vec::new(),
                    setup: serde_json::json!({}),
                })
                .unwrap();
        }
        let (second, suppressed) = Session::start(root.clone(), true).unwrap();
        assert!(suppressed.is_empty(), "A live window prevents restoration");
        second.forget(&BTreeSet::from(["9"])).unwrap();
        drop(first);
        let (another, suppressed) = Session::start(root.clone(), true).unwrap();
        assert!(
            suppressed.is_empty(),
            "The surviving window still owns a lease"
        );
        drop(another);
        drop(second);

        let (reopened, candidates) = Session::start(root.clone(), true).unwrap();
        let restored = restore_projects(&candidates);
        let ids = restored
            .iter()
            .map(|(_, record)| record.id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(ids, ["0", "8", "7", "6", "5", "4", "3"]);
        assert!(reopened
            .recent()
            .unwrap()
            .iter()
            .any(|item| item.path.ends_with("part-9.nc")));
        drop(reopened);
        for i in 0..10 {
            fs::remove_file(root.join(format!("part-{i}.nc"))).unwrap();
        }
        fs::remove_file(root.join("session.json")).unwrap();
        fs::remove_file(root.join("session.lock")).unwrap();
        fs::remove_dir(root.join("windows")).unwrap();
        fs::remove_dir(root).unwrap();
    }

    #[test]
    fn native_tab_strip_splits_creates_and_prompts_before_closing_dirty_tabs() {
        use eframe::{egui_wgpu, wgpu};
        use std::sync::Arc;
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = pollster::block_on(instance.request_adapter(&Default::default())).unwrap();
        let (device, queue) =
            pollster::block_on(adapter.request_device(&Default::default())).unwrap();
        let renderer =
            egui_wgpu::Renderer::new(&device, wgpu::TextureFormat::Rgba8Unorm, Default::default());
        let ctx = egui::Context::default();
        let mut cc = eframe::CreationContext::_new_kittest(ctx.clone());
        cc.wgpu_render_state = Some(egui_wgpu::RenderState {
            instance,
            adapter,
            device,
            queue,
            available_adapters: vec![],
            target_format: wgpu::TextureFormat::Rgba8Unorm,
            renderer: Arc::new(egui::epaint::mutex::RwLock::new(renderer)),
        });
        let mut viewer = App::new(&cc).unwrap();
        viewer.settings = Default::default();
        let mut window = Window::empty(viewer, None);
        for name in ["first.nc", "second.nc"] {
            let id = window.new_id();
            window.insert(0, Project::new(id, name, "G0 X1\nM30\n"));
        }
        window.panes[0].viewer.settings.show_toolbit_path = true;
        window.panes[0].viewer.camera.yaw = 0.75;
        let clipboard = std::cell::RefCell::new(Vec::new());
        let frame = |window: &mut Window, events| {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1280., 800.))),
                    events,
                    ..Default::default()
                },
                |ui| window.draw(ui),
            );
            let shapes = std::mem::take(&mut output.shapes);
            for command in std::mem::take(&mut output.platform_output.commands) {
                if let egui::OutputCommand::CopyText(text) = command {
                    clipboard.borrow_mut().push(text);
                }
            }
            output.drop_without_applying_deltas();
            shapes
        };
        let button = |pos, button, pressed| egui::Event::PointerButton {
            pos,
            button,
            pressed,
            modifiers: Modifiers::NONE,
        };
        let click = |window: &mut Window, pos, which| {
            for pressed in [true, false] {
                frame(
                    window,
                    vec![egui::Event::PointerMoved(pos), button(pos, which, pressed)],
                );
            }
        };
        frame(&mut window, vec![]);
        let first_tab = pos2(63., 50.);
        frame(
            &mut window,
            vec![
                egui::Event::PointerMoved(first_tab),
                button(first_tab, PointerButton::Primary, true),
            ],
        );
        let right = pos2(920., 155.);
        frame(&mut window, vec![egui::Event::PointerMoved(right)]);
        frame(
            &mut window,
            vec![button(right, PointerButton::Primary, false)],
        );
        assert_eq!(window.panes.len(), 2);
        assert_eq!(window.panes[0].viewer.main, "second.nc");
        assert_eq!(window.panes[1].viewer.main, "first.nc");
        assert!(window.panes[0].viewer.settings.show_toolbit_path);
        assert!(!window.panes[1].viewer.settings.show_toolbit_path);
        assert_eq!(window.panes[0].viewer.camera.yaw, 0.75);
        frame(&mut window, vec![]);
        let blank = pos2(902., 50.);
        click(&mut window, blank, PointerButton::Primary);
        click(&mut window, blank, PointerButton::Primary);
        assert_eq!(window.panes[1].tabs.len(), 2);
        let viewer = &mut window.panes[1].viewer;
        viewer
            .documents
            .insert(viewer.main.clone(), "Unsaved".into());
        viewer.edited(&ctx);
        let shapes = frame(&mut window, vec![]);
        let dirty_dots = shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Circle(circle) if circle.fill == Color32::from_rgb(51, 131, 91) => {
                    Some(circle.center)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(dirty_dots.len(), 1);
        assert!(dirty_dots[0].y < super::super::view::TOOLBAR_HEIGHT + tabs::HEIGHT);
        click(&mut window, pos2(800., 50.), PointerButton::Middle);
        assert!(matches!(window.pending, Some(PendingClose::Tab((1, 1)))));
        window.complete_close(PendingClose::Tab((1, 1)), &ctx);
        assert_eq!(window.panes[1].viewer.main, "first.nc");
        window.split((1, 0), &ctx);
        assert_eq!(window.panes.len(), 1);
        assert_eq!(window.panes[0].tabs.len(), 2);

        let recent_name = "production-test-file-with-a-readable-name.nc";
        let opened = chrono::NaiveDate::from_ymd_opt(2026, 11, 20)
            .unwrap()
            .and_hms_opt(12, 0, 0)
            .unwrap()
            .and_local_timezone(chrono::Local)
            .single()
            .unwrap()
            .timestamp_micros() as u64;
        window.history.rows = ["short.nc", recent_name]
            .map(|name| Recent {
                path: PathBuf::from(name),
                opened,
            })
            .to_vec();
        frame(&mut window, vec![]);
        click(&mut window, pos2(194., 18.), PointerButton::Primary);
        frame(&mut window, vec![]);
        let shapes = frame(&mut window, vec![]);
        let find_text = |caption: &str| {
            shapes.iter().find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.job.text == caption => {
                    Some((text, shape.clip_rect))
                }
                _ => None,
            })
        };
        let (date, _) = find_text("2026-11-20").unwrap();
        let (sample, _) = find_text("Sample").unwrap();
        let (file, _) = find_text(recent_name).unwrap();
        assert!(sample.pos.y < date.pos.y);
        assert!(file.pos.x > date.pos.x);
        let captions = Rect::from_min_size(sample.pos, sample.galley.size())
            .union(Rect::from_min_size(file.pos, file.galley.size()));
        let menu_right = shapes
            .iter()
            .flat_map(|shape| match &shape.shape {
                egui::Shape::Vec(parts) => parts.as_slice(),
                part => std::slice::from_ref(part),
            })
            .filter_map(|shape| match shape {
                egui::Shape::Rect(frame) if frame.rect.contains_rect(captions) => {
                    Some(frame.rect.right())
                }
                _ => None,
            })
            .reduce(f32::min)
            .unwrap();
        let right_padding = menu_right - captions.right();
        assert!(
            (5. ..=24.).contains(&right_padding),
            "Filename padding was {right_padding}"
        );
        assert!(!file.galley.elided);
        assert!(shapes.iter().any(|shape| {
            matches!(&shape.shape, egui::Shape::Path(path) if path.closed && path.points.iter().all(|point| {
                point.x >= date.pos.x && point.x < sample.pos.x
                    && point.y >= sample.pos.y && point.y <= sample.pos.y + sample.galley.size().y
            }))
        }));
        let sample_position = sample.pos + sample.galley.size() / 2.;
        let recent_position = file.pos + file.galley.size() / 2.;
        window
            .history
            .missing
            .insert(window.history.rows[1].clone());
        let after = frame(&mut window, vec![]);
        let gray_file = after
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.job.text == recent_name => Some(text),
                _ => None,
            })
            .unwrap();
        assert_eq!(gray_file.pos, file.pos);
        assert_ne!(gray_file.fallback_color, file.fallback_color);
        click(&mut window, recent_position, PointerButton::Primary);
        assert_eq!(window.panes[0].tabs.len(), 2);
        click(&mut window, sample_position, PointerButton::Primary);
        assert_eq!(window.panes[0].viewer.main, project::SAMPLE);
        assert_eq!(window.panes[0].tabs.len(), 3);
        window.complete_close(PendingClose::Tab((0, 2)), &ctx);

        let copy_tab_path = |window: &mut Window| {
            click(window, pos2(1130., 90.), PointerButton::Primary);
            frame(window, vec![]);
            click(window, first_tab, PointerButton::Secondary);
            frame(window, vec![]);
            let shapes = frame(window, vec![]);
            let position = shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.job.text == "Copy Filepath" => {
                        Some(text.pos + text.galley.size() / 2.)
                    }
                    _ => None,
                })
                .unwrap();
            click(window, position, PointerButton::Primary);
        };
        copy_tab_path(&mut window);
        assert!(clipboard.borrow().is_empty());
        let path = std::env::temp_dir().join(format!(
            "cnc-tab-path-{}-{}.nc",
            std::process::id(),
            session::now()
        ));
        fs::write(&path, "G0 X1\nM30\n").unwrap();
        let parked = window.panes[0].tabs[0].parked.as_mut().unwrap();
        parked.files.insert(
            parked.main.clone(),
            super::super::disk::File::new(path.clone(), &parked.documents[&parked.main]),
        );
        copy_tab_path(&mut window);
        assert_eq!(&*clipboard.borrow(), &[path.display().to_string()]);
        fs::remove_file(path).unwrap();
    }
}

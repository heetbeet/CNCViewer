mod disk;
mod find;
mod history;
mod project;
mod session;
mod tabs;
mod view;
mod window;

use crate::{
    camera::{dot, Camera},
    render::{Plot, PreparedPlot},
};
use cnc_core::{
    engine::{inspect, Diagnostic, Model, Options},
    error::NcError,
};
use eframe::{
    egui::{self, Color32, Key, Modifiers, PointerButton, Sense, Vec2},
    egui_wgpu::RenderState,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    time::{Duration, Instant},
};
pub use window::Window;
#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
struct Settings {
    show_variables: bool,
    variables_width: f32,
    show_rapid: bool,
    hide_after: bool,
    feed_timed: bool,
    speed: f64,
    diameter: f64,
    font_size: f32,
    dark: bool,
    editor_open: bool,
    dro_open: bool,
    machine_open: bool,
    show_grid: bool,
    show_toolbit_path: bool,
    opaque_toolbit_path: bool,
    show_points: bool,
    five_axis: bool,
    diameter_mode: bool,
    z_up: bool,
    left_width: f32,
    code_open: bool,
    mouse: [usize; 3],
    raycast: bool,
    space_mouse: bool,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            show_variables: false,
            variables_width: 157.5,
            show_rapid: true,
            hide_after: false,
            feed_timed: true,
            speed: 8.,
            diameter: 8.,
            font_size: 12.,
            dark: false,
            editor_open: true,
            dro_open: false,
            machine_open: true,
            show_grid: true,
            show_toolbit_path: false,
            opaque_toolbit_path: false,
            show_points: false,
            five_axis: true,
            diameter_mode: false,
            z_up: true,
            left_width: 450.,
            code_open: true,
            mouse: [0, 1, 2],
            raycast: true,
            space_mouse: false,
        }
    }
}
fn settings_path() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("CNCViewerNative/settings.json")
}
fn byte_at(text: &str, char_index: usize) -> usize {
    text.char_indices()
        .nth(char_index)
        .map_or(text.len(), |(byte, _)| byte)
}
enum DocumentAction {
    Open(Vec<PathBuf>),
    New,
    Sample,
}
struct Job {
    generation: u64,
    text: String,
    filename: String,
    libraries: BTreeMap<String, String>,
    cancel: Arc<AtomicBool>,
    setup: serde_json::Value,
    fit: bool,
    five_axis: bool,
    diameter: Option<f64>,
}
struct PlotResult {
    generation: u64,
    result: Result<(Model, PreparedPlot), NcError>,
    ms: f64,
    fit: bool,
}
pub struct App {
    documents: BTreeMap<String, String>,
    main: String,
    selected: String,
    disk: disk::Watcher,
    conflict_choice: Option<(String, disk::Choice)>,
    overwrite: Option<disk::Overwrite>,
    last_f5: Option<(String, Instant)>,
    title: String,
    dirty: BTreeSet<String>,
    program_setup: serde_json::Value,
    model: Arc<Model>,
    plot: Plot,
    gpu: RenderState,
    texture: egui::TextureId,
    camera: Camera,
    settings: Settings,
    status: String,
    step: usize,
    time: f64,
    running: bool,
    forward: bool,
    last_frame: Instant,
    generation: u64,
    cancel: Option<Arc<AtomicBool>>,
    sender: mpsc::Sender<Job>,
    receiver: mpsc::Receiver<PlotResult>,
    busy: bool,
    edit_deadline: Option<Instant>,
    preview_stale: bool,
    diagnostics: Vec<Diagnostic>,
    #[cfg(windows)]
    space_mouse: crate::spacemouse::SpaceMouse,
    find: find::Find,
    selected_line: usize,
    action: Option<DocumentAction>,
    project_id: String,
    opened_at: u64,
    edited_at: u64,
    resume_plot: Option<project::PlotPosition>,
    plot_paused: bool,
    settings_dirty: bool,
    capture_path: Option<PathBuf>,
    capture_frame: usize,
    popup: u8,
    fullscreen: bool,
    canvas_size: Vec2,
    scroll_line: Option<usize>,
    block_interval: f64,
    block_elapsed: f64,
    last_stop: Option<usize>,
    highlight_cache: BTreeMap<String, view::Highlight>,
    playback_rows: BTreeMap<String, view::SourceRows>,
}
impl App {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        view::fonts(&cc.egui_ctx);
        let gpu = cc
            .wgpu_render_state
            .clone()
            .ok_or("Native GPU renderer unavailable")?;
        Ok(Self::with_gpu(&cc.egui_ctx, gpu))
    }
    fn with_gpu(ctx: &egui::Context, gpu: RenderState) -> Self {
        let plot = Plot::new(gpu.device.clone(), gpu.queue.clone());
        let texture = gpu.renderer.write().register_native_texture(
            &gpu.device,
            &plot.view,
            eframe::wgpu::FilterMode::Nearest,
        );
        let mut settings: Settings = std::fs::read(settings_path())
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        if settings.mouse.iter().any(|v| *v > 2)
            || settings.mouse[0] == settings.mouse[1]
            || settings.mouse[0] == settings.mouse[2]
            || settings.mouse[1] == settings.mouse[2]
        {
            settings.mouse = [0, 1, 2];
        }
        settings.speed = settings.speed.clamp(0.1, 1024.);
        settings.diameter = settings.diameter.clamp(0.01, 10000.);
        settings.font_size = settings.font_size.clamp(9., 40.);
        let (sender, jobs) = mpsc::channel::<Job>();
        let (results, receiver) = mpsc::channel();
        let context = ctx.clone();
        let device = gpu.device.clone();
        std::thread::spawn(move || {
            while let Ok(mut job) = jobs.recv() {
                while let Ok(new) = jobs.try_recv() {
                    job = new;
                }
                let started = Instant::now();
                let options = Options {
                    capture_variables: true,
                    setup: job.setup,
                    cancel: Some(job.cancel.clone()),
                    ..Options::default()
                };
                let result =
                    inspect(&job.text, &job.filename, &job.libraries, &options).and_then(|model| {
                        let prepared = Plot::prepare(
                            &device,
                            &model,
                            job.five_axis,
                            job.diameter,
                            Some(&job.cancel),
                        )
                        .map_err(NcError::new)?;
                        Ok((model, prepared))
                    });
                if job.cancel.load(Ordering::Relaxed) {
                    continue;
                }
                if results
                    .send(PlotResult {
                        generation: job.generation,
                        result,
                        ms: started.elapsed().as_secs_f64() * 1000.,
                        fit: job.fit,
                    })
                    .is_err()
                {
                    break;
                }
                context.request_repaint();
            }
        });
        let main = project::UNTITLED.to_owned();
        let disk = disk::Watcher::new(ctx.clone());
        let documents = BTreeMap::from([(main.clone(), project::STARTER.into())]);
        let capture_path = std::env::var_os("CNC_VIEWER_CAPTURE_UI").map(PathBuf::from);
        let popup = if capture_path.is_some() {
            match std::env::var("CNC_VIEWER_CAPTURE_PANEL").as_deref() {
                Ok("mouse") => 1,
                Ok("backplot") => 2,
                _ => 0,
            }
        } else {
            0
        };
        Self {
            documents,
            main: main.clone(),
            selected: main.clone(),
            disk,
            conflict_choice: None,
            overwrite: None,
            last_f5: None,
            title: String::new(),
            dirty: BTreeSet::new(),
            program_setup: serde_json::json!({}),
            model: Arc::new(Model::default()),
            plot,
            gpu,
            texture,
            camera: Camera::default(),
            settings,
            status: String::new(),
            step: 0,
            time: 0.,
            running: false,
            forward: true,
            last_frame: Instant::now(),
            generation: 0,
            cancel: None,
            sender,
            receiver,
            busy: false,
            edit_deadline: None,
            preview_stale: false,
            diagnostics: Vec::new(),
            #[cfg(windows)]
            space_mouse: Default::default(),
            find: find::Find::default(),
            selected_line: 0,
            action: None,
            project_id: String::new(),
            opened_at: session::now(),
            edited_at: 0,
            resume_plot: None,
            plot_paused: false,
            settings_dirty: false,
            capture_path,
            capture_frame: 0,
            popup,
            fullscreen: false,
            canvas_size: Vec2::new(1280., 800. - view::TOOLBAR_HEIGHT),
            scroll_line: None,
            block_interval: 0.04,
            block_elapsed: 0.,
            last_stop: None,
            highlight_cache: BTreeMap::new(),
            playback_rows: BTreeMap::new(),
        }
    }
    fn replot(&mut self) {
        self.resume_plot = None;
        self.request_plot(true);
    }
    fn cancel_plot(&mut self) {
        if let Some(cancel) = self.cancel.take() {
            cancel.store(true, Ordering::Relaxed);
        }
        self.generation += 1;
        self.running = false;
        self.busy = false;
    }
    fn request_plot(&mut self, fit: bool) {
        self.plot_paused = false;
        self.cancel_plot();
        self.diagnostics.clear();
        self.edit_deadline = None;
        let cancel = Arc::new(AtomicBool::new(false));
        self.cancel = Some(cancel.clone());
        let libraries = self
            .documents
            .iter()
            .filter(|(n, _)| **n != self.main)
            .map(|(n, t)| (n.clone(), t.clone()))
            .collect();
        let mut setup = self.program_setup.clone();
        setup["diameterMode"] = serde_json::json!(self.settings.diameter_mode);
        let job = Job {
            generation: self.generation,
            text: self.documents.get(&self.main).cloned().unwrap_or_default(),
            filename: self.main.clone(),
            libraries,
            cancel,
            setup,
            fit,
            five_axis: self.settings.five_axis,
            diameter: self
                .settings
                .show_toolbit_path
                .then_some(self.settings.diameter),
        };
        if self.sender.send(job).is_ok() {
            self.busy = true;
            self.status = "Plotting".into()
        }
    }
    fn edited(&mut self, ctx: &egui::Context) {
        self.edited_at = session::now();
        self.playback_rows.remove(&self.selected);
        if self.disk.files.get(&self.selected).is_some_and(|file| {
            self.documents
                .get(&self.selected)
                .is_some_and(|text| file.matches_saved(text))
        }) {
            self.dirty.remove(&self.selected);
        } else {
            self.dirty.insert(self.selected.clone());
        }
        self.last_f5 = None;
        self.find.results = None;
        ctx.request_repaint();
        self.schedule_refresh(ctx, Instant::now());
    }
    fn schedule_refresh(&mut self, ctx: &egui::Context, now: Instant) {
        self.cancel_plot();
        self.diagnostics.clear();
        self.preview_stale = true;
        self.status = "Waiting to plot".into();
        let bytes: usize = self.documents.values().map(String::len).sum();
        let delay = Duration::from_millis(match bytes {
            0..=131_072 => 450,
            131_073..=1_048_576 => 900,
            _ => 1500,
        });
        self.edit_deadline = Some(now + delay);
        ctx.request_repaint_after(delay);
    }
    fn refresh_after_edit(&mut self, ctx: &egui::Context, now: Instant, focused: bool) {
        if let Some(deadline) = self.edit_deadline.filter(|_| focused) {
            if now >= deadline {
                self.request_plot(false);
            } else {
                ctx.request_repaint_after(deadline - now);
            }
        }
    }
    #[cfg(test)]
    fn open(&mut self, paths: Vec<PathBuf>, ctx: &egui::Context) {
        let Some(first) = paths.first() else { return };
        let result = paths
            .iter()
            .map(|path| read_document(path))
            .collect::<Result<Vec<_>, _>>();
        match result {
            Ok(documents)
                if documents
                    .iter()
                    .map(|(name, _, _)| name)
                    .collect::<BTreeSet<_>>()
                    .len()
                    == paths.len() =>
            {
                let main = first
                    .file_name()
                    .expect("validated document path")
                    .to_string_lossy()
                    .into_owned();
                self.forget_editor_history(ctx);
                self.documents.clear();
                self.disk.files.clear();
                for (name, text, file) in documents {
                    self.disk.files.insert(name.clone(), file);
                    self.documents.insert(name, text);
                }
                self.reset_disk_choices();
                self.selected = main.clone();
                self.main = main;
                self.dirty.clear();
                self.program_setup = serde_json::json!({});
                self.replot();
            }
            Ok(_) => self.status = "Program filenames must be unique".into(),
            Err(error) => self.status = error.to_string(),
        }
    }
    fn open_dialog(&mut self) {
        if let Some(paths) = rfd::FileDialog::new()
            .set_title("Open G-code")
            .add_filter("G-code", &["nc", "tap", "cnc", "gcode", "ngc", "txt", "*"])
            .pick_files()
        {
            self.request_document_action(DocumentAction::Open(paths));
        }
    }
    fn add_libraries(&mut self) {
        if let Some(paths) = rfd::FileDialog::new()
            .set_title("Add subprograms")
            .pick_files()
        {
            let files = paths
                .into_iter()
                .map(|path| read_document(&path))
                .collect::<Result<Vec<_>, Box<dyn std::error::Error>>>();
            match files {
                Ok(files) => {
                    let mut names = BTreeSet::new();
                    if files.iter().any(|(name, _, _)| {
                        self.documents.contains_key(name) || !names.insert(name)
                    }) {
                        self.status = "Program filenames must be unique".into();
                        return;
                    }
                    for (name, text, file) in files {
                        self.disk.files.insert(name.clone(), file);
                        self.documents.insert(name, text);
                    }
                    self.disk.invalidate();
                    self.replot();
                }
                Err(error) => self.status = error.to_string(),
            }
        }
    }
    fn load_machine_setup(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("Machine setup", &["json"])
            .pick_file()
        else {
            return;
        };
        let result = (|| -> Result<serde_json::Value, Box<dyn std::error::Error>> {
            let text = crate::cli::input(path.to_str().ok_or("Invalid setup filename")?)?;
            let setup: serde_json::Value =
                serde_json::from_str(text.trim_start_matches('\u{feff}'))?;
            if !setup.is_object() {
                return Err("Machine setup must be a JSON object".into());
            }
            cnc_core::engine::validate_setup(setup.clone())?;
            Ok(setup)
        })();
        match result {
            Ok(setup) => {
                self.program_setup = setup;
                self.replot();
            }
            Err(error) => self.status = error.to_string(),
        }
    }
    fn request_document_action(&mut self, action: DocumentAction) {
        self.action = Some(action);
    }
    #[cfg(test)]
    fn apply_document_action(&mut self, action: DocumentAction, ctx: &egui::Context) {
        let (name, text, setup) = match action {
            DocumentAction::Open(paths) => return self.open(paths, ctx),
            DocumentAction::New => ("New GCode File.nc", "", serde_json::json!({})),
            DocumentAction::Sample => (
                "CNCViewerDemo.nc",
                include_str!("../../../examples/CNCViewerDemo.nc"),
                serde_json::json!({}),
            ),
        };
        self.forget_editor_history(ctx);
        self.documents = BTreeMap::from([(name.into(), text.into())]);
        self.disk.files.clear();
        self.reset_disk_choices();
        self.main = name.into();
        self.selected = self.main.clone();
        self.dirty.clear();
        self.program_setup = setup;
        self.replot();
    }
    fn seek(&mut self, step: usize) {
        self.last_stop = None;
        self.step = step.min(self.model.executions.len().saturating_sub(1));
        if let Some(state) = self.model.executions.get(self.step) {
            self.time = state.elapsed / 1000.;
            self.scroll_line = Some(state.line);
            self.selected_line = state.line;
            if self.documents.contains_key(&state.source) {
                self.selected = state.source.clone()
            }
        }
        self.running = false;
    }
    fn play(&mut self, forward: bool) {
        if self.model.executions.is_empty() || self.preview_stale {
            return;
        }
        if forward && self.step + 1 >= self.model.executions.len() {
            self.seek(0)
        } else if !forward && self.step == 0 {
            self.seek(self.model.executions.len() - 1)
        }
        self.block_elapsed = 0.;
        self.forward = forward;
        self.running = true;
        self.last_frame = Instant::now();
    }
    fn faster(&mut self, forward: bool) {
        if self.settings.feed_timed {
            self.settings.speed = (self.settings.speed * 2.).min(1024.);
        } else {
            self.block_interval = (self.block_interval - 0.01).max(0.005);
        }
        self.play(forward);
    }
    fn local_id(&self, name: impl std::hash::Hash + std::fmt::Debug) -> egui::Id {
        egui::Id::unique((&self.project_id, name))
    }
    fn editor_id_for(&self, name: &str) -> egui::Id {
        self.local_id(("cnc-editor", name))
    }
    fn editor_id(&self) -> egui::Id {
        self.editor_id_for(&self.selected)
    }
    #[cfg(test)]
    fn forget_editor_history(&mut self, ctx: &egui::Context) {
        self.find.results = None;
        self.highlight_cache.clear();
        ctx.data_mut(|data| {
            for name in self.documents.keys() {
                data.remove::<egui::text_edit::TextEditState>(self.editor_id_for(name));
            }
        });
    }
    fn edit_history(&mut self, ctx: &egui::Context, redo: bool) {
        let id = self.editor_id();
        let Some(mut state) = egui::text_edit::TextEditState::load(ctx, id) else {
            return;
        };
        let Some(text) = self.documents.get_mut(&self.selected) else {
            return;
        };
        let current = (state.cursor.char_range().unwrap_or_default(), text.clone());
        let mut undoer = state.undoer();
        let restored = if redo {
            undoer.redo(&current)
        } else {
            undoer.undo(&current)
        };
        if let Some((cursor, restored)) = restored {
            let changed = *text != *restored;
            if changed {
                text.clone_from(restored);
                self.edited(ctx);
            }
            state.cursor.set_char_range(Some(*cursor));
            state.set_undoer(undoer);
            state.store(ctx, id);
        }
    }
    fn find_id(&self) -> egui::Id {
        self.local_id("editor-find")
    }
    fn open_find(&mut self, ctx: &egui::Context, replace: bool) {
        let id = self.editor_id();
        if ctx.memory(|memory| memory.has_focus(id)) {
            if let Some(range) = egui::text_edit::TextEditState::load(ctx, id)
                .and_then(|state| state.cursor.char_range())
                .filter(|range| !range.is_empty())
            {
                if let Some(text) = self.documents.get(&self.selected) {
                    let range = range.as_sorted_char_range();
                    let start = byte_at(text, range.start.into());
                    let end = byte_at(text, range.end.into());
                    let selection = &text[start..end];
                    if !selection.contains('\n') && self.find.query != selection {
                        self.find.query = selection.to_owned();
                        self.find.results = None;
                    }
                }
            }
        }
        self.find.open = true;
        self.find.replace_open = replace;
        self.find.reveal = true;
        self.settings.editor_open = true;
        self.settings.code_open = true;
        let id = self.find_id();
        let mut state = egui::text_edit::TextEditState::load(ctx, id).unwrap_or_default();
        state
            .cursor
            .set_char_range(Some(egui::text::CCursorRange::select_all_str(
                &self.find.query,
            )));
        state.store(ctx, id);
        ctx.memory_mut(|memory| memory.request_focus(id));
    }
    fn close_find(&mut self, ctx: &egui::Context) {
        self.find.open = false;
        self.find.reveal = false;
        ctx.memory_mut(|memory| memory.request_focus(self.editor_id()));
    }
    fn refresh_find(&mut self, ctx: &egui::Context) {
        if self
            .find
            .results
            .as_ref()
            .is_some_and(|r| r.source == self.selected)
        {
            return;
        }
        let Some(text) = self.documents.get(&self.selected) else {
            return;
        };
        let anchor = egui::text_edit::TextEditState::load(ctx, self.editor_id())
            .and_then(|state| state.cursor.char_range())
            .map_or(0, |range| {
                byte_at(text, range.as_sorted_char_range().start.into())
            });
        self.find.refresh(&self.selected, text, anchor);
    }
    fn replace_matches(&mut self, ctx: &egui::Context, all: bool) {
        self.refresh_find(ctx);
        let id = self.editor_id();
        let mut state = egui::text_edit::TextEditState::load(ctx, id).unwrap_or_default();
        let Some(text) = self.documents.get_mut(&self.selected) else {
            return;
        };
        let Some((replacement, next)) = self.find.replacement_text(text, all) else {
            return;
        };
        if *text == replacement {
            if !all {
                self.find.advance(false);
            }
            return;
        }
        let current = (
            state.cursor.char_range().unwrap_or_default(),
            std::mem::replace(text, replacement),
        );
        let caret = text[..next].chars().count();
        let range = egui::text::CCursorRange::one(egui::text::CCursor::new(caret));
        let mut undoer = state.undoer();
        undoer.add_undo(&current);
        undoer.add_undo(&(range, text.clone()));
        state.set_undoer(undoer);
        state.cursor.set_char_range(Some(range));
        state.store(ctx, id);
        self.edited(ctx);
        self.refresh_find(ctx);
        self.find.reveal = !all;
    }
    fn keyboard(&mut self, ctx: &egui::Context) {
        if self.overwrite.is_some() {
            return;
        }
        let refresh = ctx.input_mut(|i| {
            let pressed = i.events.iter().any(|event| matches!(event, egui::Event::Key { key: Key::F5, pressed: true, repeat: false, modifiers, .. } if modifiers.is_none()));
            i.consume_key(Modifiers::NONE, Key::F5);
            pressed
        });
        if refresh {
            self.refresh_key(ctx, Instant::now());
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::O)) {
            self.open_dialog()
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::S)) {
            self.save(true)
        } else if ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::S)) {
            self.save(false)
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::N)) {
            self.request_document_action(DocumentAction::New)
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::Enter)) {
            self.replot()
        }
        let replace = ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::H));
        if replace || ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::F)) {
            self.open_find(ctx, replace);
        }
        if self.find.open && self.settings.editor_open && self.settings.code_open {
            let previous = ctx.input_mut(|i| i.consume_key(Modifiers::SHIFT, Key::F3));
            if previous || ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::F3)) {
                self.refresh_find(ctx);
                self.find.advance(previous);
            }
        }
        if !egui::Popup::is_any_open(ctx)
            && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape))
        {
            if self.find.open && self.settings.editor_open && self.settings.code_open {
                self.close_find(ctx);
            } else if ctx.memory(|m| m.has_focus(self.editor_id())) {
                ctx.memory_mut(|m| m.surrender_focus(self.editor_id()));
            } else if self.popup != 0 {
                self.popup = 0;
            } else {
                self.settings.editor_open = !self.settings.editor_open;
            }
        }
        if ctx.memory(|memory| memory.has_focus(self.local_id("program-progress"))) {
            let destination = ctx.input_mut(|input| {
                if input.consume_key(Modifiers::NONE, Key::Home) {
                    Some(0)
                } else if input.consume_key(Modifiers::NONE, Key::End) {
                    Some(max_step(&self.model))
                } else if input.consume_key(Modifiers::NONE, Key::ArrowLeft) {
                    Some(self.step.saturating_sub(1))
                } else if input.consume_key(Modifiers::NONE, Key::ArrowRight) {
                    Some(self.step + 1)
                } else {
                    None
                }
            });
            if let Some(step) = destination {
                self.seek(step);
            }
        }
        if !ctx.egui_wants_keyboard_input() {
            if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Space)) {
                if self.running {
                    self.running = false
                } else {
                    self.play(true)
                }
            }
            if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::ArrowRight)) {
                self.seek(self.step + 1)
            }
            if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::ArrowLeft)) {
                self.seek(self.step.saturating_sub(1))
            }
            if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Home)) {
                self.camera = Camera::default();
                self.camera.fit(
                    self.model.bounds,
                    self.canvas_size.x as f64,
                    self.canvas_size.y as f64,
                );
            }
        }
    }
    fn interrupt_for_typing(&mut self, ctx: &egui::Context) {
        if !(self.busy || self.running)
            || self.overwrite.is_some()
            || !ctx.memory(|mem| mem.has_focus(self.editor_id()))
        {
            return;
        }
        let editing = ctx.input(|input| {
            input.events.iter().any(|event| match event {
                egui::Event::Text(_)
                | egui::Event::Paste(_)
                | egui::Event::Cut
                | egui::Event::Ime(_) => true,
                egui::Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } => match key {
                    Key::Backspace | Key::Delete => true,
                    Key::Enter | Key::Tab => !modifiers.command,
                    Key::Z | Key::Y => modifiers.command,
                    _ => false,
                },
                _ => false,
            })
        });
        if editing {
            self.schedule_refresh(ctx, Instant::now());
        }
    }
    fn receive(&mut self) {
        while let Ok(PlotResult {
            generation,
            result,
            ms,
            fit,
        }) = self.receiver.try_recv()
        {
            if generation != self.generation {
                continue;
            }
            self.busy = false;
            match result {
                Ok((model, prepared)) => {
                    self.preview_stale = false;
                    self.diagnostics
                        .extend(model.diagnostics.iter().chain(&model.warnings).cloned());
                    self.diagnostics
                        .sort_by(|a, b| a.source.cmp(&b.source).then(a.line.cmp(&b.line)));
                    self.status = format!("{} moves · {ms:.0} ms", model.moves.len());
                    if let Some(error) = &prepared.trail_error {
                        self.settings.show_toolbit_path = false;
                        self.status = error.clone();
                    }
                    let resume = self.resume_plot.take();
                    let fit_camera = fit || self.model.bounds.is_none() && resume.is_none();
                    self.plot.install(prepared);
                    self.model = Arc::new(model);
                    if let Err(error) = self
                        .plot
                        .set_five_axis(&self.model, self.settings.five_axis)
                    {
                        self.status = error;
                    }
                    let last = self.model.executions.len().saturating_sub(1);
                    if let Some(resume) = resume {
                        self.step = resume.step.min(last);
                        let start = self
                            .model
                            .executions
                            .get(self.step)
                            .map_or(0., |state| state.elapsed / 1000.);
                        let end = self
                            .model
                            .executions
                            .get(self.step + 1)
                            .map_or(start, |state| state.elapsed / 1000.);
                        self.time = resume.time.clamp(start, end);
                        self.selected_line = resume.line;
                        self.scroll_line = Some(resume.line);
                    } else if fit {
                        self.seek(last);
                    } else {
                        self.step = last;
                        self.time = self
                            .model
                            .executions
                            .last()
                            .map_or(0., |state| state.elapsed / 1000.);
                    }
                    if fit_camera {
                        self.camera.fit(
                            self.model.bounds,
                            self.canvas_size.x as f64,
                            self.canvas_size.y as f64,
                        );
                    }
                }
                Err(mut error) => {
                    if let Some(location) = error.location.take() {
                        self.status = "Plot stopped".into();
                        self.diagnostics.push(Diagnostic {
                            source: location.source,
                            line: location.line,
                            step: 0,
                            error,
                        });
                    } else {
                        self.status = error.to_string();
                    }
                }
            }
        }
    }
    fn advance(&mut self) {
        let delta = self.last_frame.elapsed().as_secs_f64();
        self.last_frame = Instant::now();
        if !self.running {
            return;
        }
        if self.settings.feed_timed {
            let previous = self.time;
            let duration = self
                .model
                .executions
                .last()
                .map_or(0., |s| s.elapsed / 1000.);
            self.time = (self.time
                + delta * self.settings.speed * if self.forward { 1. } else { -1. })
            .clamp(0., duration);
            let stops = self
                .model
                .events
                .iter()
                .filter(|e| e["type"] == "program-stop")
                .filter_map(|e| {
                    let step = e["step"].as_u64()? as usize;
                    let time = self.model.executions.get(step.checked_sub(1)?)?.elapsed / 1000.;
                    Some((step, time))
                });
            let candidates: Vec<_> = stops
                .filter(|(step, time)| {
                    (self.last_stop != Some(*step) || (previous - time).abs() > 1e-9)
                        && if self.forward {
                            *time >= previous
                                && *time <= self.time
                                && (*time != previous || *step > self.step)
                        } else {
                            *time <= previous
                                && *time >= self.time
                                && (*time != previous || *step <= self.step + 1)
                        }
                })
                .collect();
            let stop = if self.forward {
                candidates.first()
            } else {
                candidates.last()
            }
            .copied();
            if let Some((step, time)) = stop {
                self.time = time;
                self.last_stop = Some(step);
                self.running = false;
            }
            self.step = self
                .model
                .executions
                .partition_point(|s| s.elapsed / 1000. <= self.time)
                .min(self.model.executions.len().saturating_sub(1));
            if let Some((step, _)) = stop {
                self.step = step - 1;
            } else if !self.forward && self.time == 0. {
                self.step = 0;
                self.running = false;
            } else if self.forward && self.time >= duration {
                self.running = false;
            }
        } else {
            self.block_elapsed += delta;
            let amount = (self.block_elapsed / self.block_interval).floor() as usize;
            if amount == 0 {
                return;
            }
            self.block_elapsed -= amount as f64 * self.block_interval;
            let previous_step = self.step;
            self.step = if self.forward {
                (self.step.saturating_add(amount))
                    .min(self.model.executions.len().saturating_sub(1))
            } else {
                self.step.saturating_sub(amount)
            };
            let stops = self
                .model
                .events
                .iter()
                .filter(|e| e["type"] == "program-stop")
                .filter_map(|e| {
                    e["step"]
                        .as_u64()
                        .and_then(|s| s.checked_sub(1))
                        .map(|s| s as usize)
                })
                .filter(|step| {
                    if self.forward {
                        *step > previous_step && *step <= self.step
                    } else {
                        *step < previous_step && *step >= self.step
                    }
                });
            if let Some(step) = if self.forward {
                stops.min()
            } else {
                stops.max()
            } {
                self.step = step;
                self.running = false;
            }
            if let Some(state) = self.model.executions.get(self.step) {
                self.time = state.elapsed / 1000.
            }
            if self.step == 0 || self.step + 1 >= self.model.executions.len() {
                self.running = false;
            }
        }
        if let Some(state) = self.model.executions.get(self.step) {
            if self.selected_line != state.line || self.selected != state.source {
                self.scroll_line = Some(state.line);
            }
            self.selected_line = state.line;
            if self.documents.contains_key(&state.source) {
                self.selected = state.source.clone()
            }
        }
    }
    fn active_movement(&self) -> Option<&cnc_core::engine::Movement> {
        let state = self.model.executions.get(self.step)?;
        if !self.settings.feed_timed || self.time == state.elapsed / 1000. {
            return None;
        }
        let index = self
            .model
            .moves
            .partition_point(|m| m.start_time <= self.time)
            .checked_sub(1)?;
        self.model
            .moves
            .get(index)
            .filter(|movement| movement.step == state.step)
    }
    fn stationary_state(&self) -> Option<&cnc_core::engine::State> {
        let state = self.model.executions.get(self.step)?;
        if self.settings.feed_timed && self.time != state.elapsed / 1000. && self.step > 0 {
            return self.model.executions.get(self.step - 1);
        }
        Some(state)
    }
    fn active_segment(&self, movement: &cnc_core::engine::Movement) -> (usize, f64) {
        if movement.seconds <= 0. || self.time >= movement.start_time + movement.seconds {
            return (movement.segment_end, 0.);
        }
        let progress = ((self.time - movement.start_time) / movement.seconds).clamp(0., 1.);
        if movement.length == 0. && movement.segment_end > movement.segment_start {
            let cursor = progress * (movement.segment_end - movement.segment_start) as f64;
            return (
                movement.segment_start + cursor.floor() as usize,
                cursor.fract(),
            );
        }
        let mut remaining = movement.length * progress;
        for index in movement.segment_start..movement.segment_end {
            let segment = &self.model.segments[index];
            let length = cnc_core::geometry::distance(&segment.start, &segment.end);
            if length > 0. && remaining < length {
                return (index, (remaining / length).clamp(0., 1.));
            }
            remaining -= length;
        }
        (movement.segment_end, 0.)
    }
    fn position(&self) -> Option<[f64; 3]> {
        let state = self.stationary_state()?;
        let Some(movement) = self.active_movement() else {
            return Some(state.position);
        };
        let (index, fraction) = self.active_segment(movement);
        if index < movement.segment_end {
            let segment = &self.model.segments[index];
            Some(cnc_core::geometry::interpolate(
                segment.start,
                segment.end,
                fraction,
            ))
        } else {
            Some(movement.end)
        }
    }
    fn tool_rotation(&self) -> [f64; 3] {
        if !self.settings.five_axis {
            return [0.; 3];
        }
        if let Some(movement) = self.active_movement() {
            let (index, fraction) = self.active_segment(movement);
            if index < movement.segment_end {
                let segment = &self.model.segments[index];
                return cnc_core::geometry::interpolate(
                    segment.start_rotation,
                    segment.rotation,
                    fraction,
                );
            }
            if movement.segment_end > movement.segment_start {
                return self.model.segments[movement.segment_end - 1].rotation;
            }
        }
        self.stationary_state()
            .map_or([0.; 3], |state| state.rotation)
    }
    fn dro(&self) -> ([f64; 6], bool) {
        let Some(state) = self.model.executions.get(self.step) else {
            return ([0.; 6], false);
        };
        let movement = self.active_movement();
        let coordinate_state = if movement.is_some() {
            state
        } else {
            self.stationary_state().unwrap_or(state)
        };
        let mut axes = coordinate_state
            .part_position
            .unwrap_or(coordinate_state.axes);
        if let Some(movement) = movement {
            let point = self.position().unwrap_or(state.position);
            let angle = (-state.dro_angle).to_radians();
            let (sin, cos) = angle.sin_cos();
            let x = point[0] - state.position[0];
            let y = point[1] - state.position[1];
            axes[0] += x * cos - y * sin;
            axes[1] += x * sin + y * cos;
            axes[2] += point[2] - state.position[2];
            let progress = if movement.seconds > 0. {
                ((self.time - movement.start_time) / movement.seconds).clamp(0., 1.)
            } else {
                1.
            };
            for (i, value) in axes.iter_mut().enumerate().skip(3) {
                let axis = movement.start_axes[i]
                    + (movement.end_axes[i] - movement.start_axes[i]) * progress;
                *value += axis - state.axes[i];
            }
        }
        let inch = state.units == "inch";
        if inch {
            for value in &mut axes[..3] {
                *value /= 25.4;
            }
        }
        (axes, inch)
    }
    fn orbit_drag(&mut self, response: &egui::Response, button: PointerButton) {
        if response.dragged_by(button) && response.ctx.current_pass_index() == 0 {
            let motion = response.drag_motion();
            self.camera.yaw -= f64::from(motion.x) * 0.008;
            self.camera.pitch = (self.camera.pitch + f64::from(motion.y) * 0.008)
                .clamp(-std::f64::consts::FRAC_PI_2, std::f64::consts::FRAC_PI_2);
        }
    }
    fn canvas(&mut self, ui: &mut egui::Ui, sidebar_width: f32) {
        let available = ui.available_size().max(Vec2::splat(1.));
        let (rect, response) = ui.allocate_exact_size(available, Sense::click_and_drag());
        let previous = self.canvas_size;
        self.canvas_size = rect.size();
        if previous.y > 0. && previous.y != rect.height() {
            self.camera.scale *= previous.y as f64 / rect.height() as f64;
        }
        let delta = ui.input(|i| i.pointer.delta());
        let blocked = ui
            .input(|i| i.pointer.hover_pos())
            .is_some_and(|p| p.x < rect.min.x + sidebar_width);

        let buttons = [
            PointerButton::Primary,
            PointerButton::Middle,
            PointerButton::Secondary,
        ];
        if !blocked && response.dragged_by(buttons[self.settings.mouse[0]]) {
            self.camera.pan(delta.x as f64, delta.y as f64)
        }
        if !blocked {
            self.orbit_drag(&response, buttons[self.settings.mouse[2]]);
        }
        if !blocked && response.dragged_by(buttons[self.settings.mouse[1]]) {
            self.camera.zoom((delta.y as f64 * 0.01).exp(), [0.; 2]);
        }
        if !blocked && response.hovered() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll != 0. {
                let p =
                    ui.input(|i| i.pointer.hover_pos()).unwrap_or(rect.center()) - rect.center();
                self.camera
                    .zoom((-scroll as f64 * 0.002).exp(), [p.x as f64, p.y as f64]);
            }
        }
        self.camera.z_up = self.settings.z_up;
        if self.settings.raycast && !blocked && (response.clicked() || response.double_clicked()) {
            if let Some(mouse) = response.interact_pointer_pos() {
                let p = mouse - rect.center();
                let mut nearest = None;
                let mut distance = 12f64;
                let mut front = f64::NEG_INFINITY;
                let depth_axis = self.camera.depth_axis();
                for segment in &self.model.segments {
                    if segment.rapid && !self.settings.show_rapid {
                        continue;
                    }
                    let end = if self.settings.five_axis {
                        cnc_core::geometry::rotate_point(segment.end, segment.rotation)
                    } else {
                        segment.end
                    };
                    let q = self.camera.project(end);
                    let d = (q[0] - p.x as f64).hypot(q[1] - p.y as f64);
                    if d > distance + 1e-6 {
                        continue;
                    }
                    let depth = dot(end, depth_axis);
                    if d < distance - 1e-6 || (d - distance).abs() <= 1e-6 && depth > front {
                        distance = d;
                        front = depth;
                        nearest = Some((segment.step, end));
                    }
                }
                if let Some((step, point)) = nearest {
                    self.seek(step.saturating_sub(1));
                    if response.double_clicked() {
                        self.camera.center = point;
                    }
                }
            }
        }
        if let Err(error) = self
            .plot
            .set_five_axis(&self.model, self.settings.five_axis)
        {
            self.status = error;
        }
        let dpi = ui.ctx().pixels_per_point();
        let size = [
            (rect.width() * dpi).round().max(1.) as u32,
            (rect.height() * dpi).round().max(1.) as u32,
        ];
        let mut partial = None;
        let visible = if self.settings.hide_after {
            if let Some(movement) = self.active_movement() {
                let (index, fraction) = self.active_segment(movement);
                if index < movement.segment_end && fraction > 0. {
                    partial = Some((&self.model.segments[index], fraction));
                }
                index
            } else {
                self.stationary_state().map_or(0, |s| s.segment_count)
            }
        } else {
            self.model.segments.len()
        };
        if self.settings.show_toolbit_path {
            if let Err(error) = self.plot.trail(&self.model, self.settings.diameter) {
                self.status = error;
                self.settings.show_toolbit_path = false;
            }
        }
        self.plot.partial_trail(if self.settings.show_toolbit_path {
            partial.map(|(segment, fraction)| (segment, visible, fraction))
        } else {
            None
        });
        let rotation = self.tool_rotation();
        let motion = partial.map(|(segment, fraction)| {
            let end = cnc_core::geometry::interpolate(segment.start, segment.end, fraction);
            let endpoints = if self.settings.five_axis {
                [
                    cnc_core::geometry::rotate_point(segment.start, segment.start_rotation),
                    cnc_core::geometry::rotate_point(end, rotation),
                ]
            } else {
                [segment.start, end]
            };
            (endpoints, segment.rapid)
        });
        let tool = self.position().map(|point| {
            let length = (self.camera.scale * rect.height() as f64 / 20.).max(2.);
            let top = [point[0], point[1], point[2] + length];
            [point, top].map(|end| cnc_core::geometry::rotate_point(end, rotation))
        });
        let resized = self.plot.render(
            size,
            dpi as f64,
            &self.camera,
            crate::render::Draw {
                rapid: self.settings.show_rapid,
                grid: self.settings.show_grid,
                trail: crate::trail::Mode::from_options(
                    self.settings.show_toolbit_path,
                    self.settings.opaque_toolbit_path,
                ),
                points: self.settings.show_points,
                segments: visible,
                live: crate::render::LiveDraw { motion, tool },
            },
        );
        if resized {
            self.gpu
                .renderer
                .write()
                .update_egui_texture_from_wgpu_texture(
                    &self.gpu.device,
                    &self.plot.view,
                    eframe::wgpu::FilterMode::Nearest,
                    self.texture,
                )
        }
        ui.painter().image(
            self.texture,
            rect,
            egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1., 1.)),
            Color32::WHITE,
        );
    }
}
fn read_document(
    path: &std::path::Path,
) -> Result<(String, String, disk::File), Box<dyn std::error::Error>> {
    let path = std::path::absolute(path)?;
    let name = path
        .file_name()
        .ok_or("Invalid input filename")?
        .to_string_lossy()
        .into_owned();
    let snapshot = disk::Snapshot::read(&path);
    let mut tracking = disk::File::new(path, "");
    tracking.observe(&snapshot);
    tracking.saved = snapshot.version;
    let text = snapshot.text?.ok_or("File is missing on disk")?;
    Ok((name, text, tracking))
}
fn max_step(model: &Model) -> usize {
    model.executions.len().saturating_sub(1)
}
impl App {
    fn draw_ui(&mut self, ui: &mut egui::Ui, active: bool, tabs: &mut tabs::Strip<'_>) {
        let focused = ui.input(|input| input.focused);
        if !focused && self.busy && self.preview_stale && self.capture_path.is_none() {
            self.cancel_plot();
            self.edit_deadline = Some(Instant::now());
        }
        if active {
            self.interrupt_for_typing(ui.ctx());
        }
        self.advance();
        if active {
            self.keyboard(ui.ctx());
        }
        #[cfg(windows)]
        if self.settings.space_mouse {
            if focused && active {
                ui.ctx()
                    .request_repaint_after(self.space_mouse.update(&mut self.camera));
            }
        } else {
            self.space_mouse = Default::default();
        }
        let before = serde_json::to_string(&self.settings).unwrap_or_default();
        ui.style_mut().visuals = if self.settings.dark {
            egui::Visuals::dark()
        } else {
            egui::Visuals::light()
        };
        self.near_view(ui, Some(tabs));
        self.watch_disk(ui.ctx(), focused, Instant::now());
        self.refresh_after_edit(ui.ctx(), Instant::now(), focused);
        let generation = self.busy;
        if focused || self.capture_path.is_some() {
            self.receive();
        }
        if generation && !self.busy {
            ui.ctx().request_repaint();
        }
        if active {
            self.update_title(ui.ctx());
        }
        self.overwrite_modal(ui.ctx());
        if before != serde_json::to_string(&self.settings).unwrap_or_default() {
            self.settings_dirty = true;
        }
        if !self.busy && !self.model.executions.is_empty() && self.capture_path.is_some() {
            self.capture_frame += 1;
            if self.capture_frame == 4 {
                let path = self.capture_path.take().unwrap_or_default();
                let ctx = ui.ctx().clone();
                ui.ctx().request_screenshot(move |image| {
                    let result = (|| -> Result<(), Box<dyn std::error::Error>> {
                        if let Some(parent) = path.parent() {
                            std::fs::create_dir_all(parent)?;
                        }
                        let pixels: Vec<u8> =
                            image.pixels.iter().flat_map(|c| c.to_array()).collect();
                        let mut encoder = png::Encoder::new(
                            std::io::BufWriter::new(std::fs::File::create(path)?),
                            image.size[0] as u32,
                            image.size[1] as u32,
                        );
                        encoder.set_color(png::ColorType::Rgba);
                        encoder.set_depth(png::BitDepth::Eight);
                        encoder.write_header()?.write_image_data(&pixels)?;
                        Ok(())
                    })();
                    if let Err(e) = result {
                        eprintln!("UI capture failed: {e}");
                        std::process::exit(1)
                    }
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                });
            }
            ui.ctx().request_repaint();
        }
        if self.running {
            ui.ctx().request_repaint();
        }
    }
    fn save_settings(&mut self) {
        if self.settings_dirty {
            let path = settings_path();
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if let Ok(bytes) = serde_json::to_vec(&self.settings) {
                let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
                if std::fs::write(&temporary, bytes).is_ok() {
                    let _ = std::fs::rename(temporary, path);
                }
            }
        }
    }
}

impl Drop for App {
    fn drop(&mut self) {
        self.cancel_plot();
        self.gpu.renderer.write().free_texture(&self.texture);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_controls_change_state_and_pausing_keeps_tool_position() {
        use eframe::{egui_wgpu, wgpu};
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = pollster::block_on(instance.request_adapter(&Default::default())).unwrap();
        let (device, queue) =
            pollster::block_on(adapter.request_device(&Default::default())).unwrap();
        let renderer =
            egui_wgpu::Renderer::new(&device, wgpu::TextureFormat::Rgba8Unorm, Default::default());
        let ctx = egui::Context::default();
        let mut cc = eframe::CreationContext::_new_kittest(ctx.clone());
        cc.wgpu_render_state = Some(RenderState {
            instance,
            adapter,
            device,
            queue,
            available_adapters: vec![],
            target_format: wgpu::TextureFormat::Rgba8Unorm,
            renderer: Arc::new(egui::epaint::mutex::RwLock::new(renderer)),
        });
        let mut app = App::new(&cc).unwrap();
        app.settings = Settings::default();
        app.replot();
        let finish_plot = |app: &mut App| {
            for _ in 0..200 {
                app.receive();
                if !app.busy {
                    return;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            panic!("Plot did not finish");
        };
        finish_plot(&mut app);
        assert!(!app.model.executions.is_empty());
        let frame = |app: &mut App, events| {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        Vec2::new(1280., 800.),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| {
                    app.interrupt_for_typing(ui.ctx());
                    app.keyboard(ui.ctx());
                    app.near_view(ui, None);
                    if let Some(action) = app.action.take() {
                        app.apply_document_action(action, ui.ctx());
                    }
                    app.overwrite_modal(ui.ctx());
                },
            );
            let shapes = std::mem::take(&mut output.shapes);
            output.drop_without_applying_deltas();
            shapes
        };
        frame(&mut app, vec![]);
        let click = |app: &mut App, x, y| {
            let pos = egui::pos2(x, y);
            for pressed in [true, false] {
                frame(
                    app,
                    vec![
                        egui::Event::PointerMoved(pos),
                        egui::Event::PointerButton {
                            pos,
                            button: PointerButton::Primary,
                            pressed,
                            modifiers: Modifiers::NONE,
                        },
                    ],
                );
            }
        };
        click(&mut app, 438., 84.);
        assert!(
            app.settings.show_variables,
            "Variables handle must work above the scene"
        );
        frame(&mut app, vec![]);
        let divider = egui::pos2(450. - app.settings.variables_width, 200.);
        frame(
            &mut app,
            vec![
                egui::Event::PointerMoved(divider),
                egui::Event::PointerButton {
                    pos: divider,
                    button: PointerButton::Primary,
                    pressed: true,
                    modifiers: Modifiers::NONE,
                },
            ],
        );
        let left = divider - Vec2::new(42., 0.);
        frame(&mut app, vec![egui::Event::PointerMoved(left)]);
        frame(
            &mut app,
            vec![egui::Event::PointerButton {
                pos: left,
                button: PointerButton::Primary,
                pressed: false,
                modifiers: Modifiers::NONE,
            }],
        );
        assert_eq!(app.settings.variables_width, 199.5);
        assert_eq!(app.settings.left_width, 450.);
        click(&mut app, 262.5, 84.);
        assert!(!app.settings.show_variables);
        frame(&mut app, vec![]);
        click(&mut app, 438., 84.);
        assert!(app.settings.show_variables);
        assert_eq!(app.settings.variables_width, 199.5);
        click(&mut app, 1187., 773.);
        assert_eq!(app.popup, 2, "Backplot popup must open");
        let controls = frame(&mut app, vec![]);
        let mut arrows: Vec<_> = controls
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Rect(r)
                    if r.rect.size() == Vec2::new(14., 10.)
                        && r.fill == Color32::from_gray(215) =>
                {
                    Some(r.rect.center())
                }
                _ => None,
            })
            .collect();
        arrows.sort_by(|a, b| a.y.total_cmp(&b.y));
        assert_eq!(arrows.len(), 4);
        let occlude = controls
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(t) if t.galley.job.text == "Occlude" => {
                    Some(t.pos + t.galley.size() / 2.)
                }
                _ => None,
            })
            .expect("Occlude checkbox must be visible");
        assert!((arrows[0].y..arrows[1].y).contains(&occlude.y));
        click(&mut app, occlude.x, occlude.y);
        assert!(app.settings.opaque_toolbit_path);
        click(&mut app, occlude.x, occlude.y);
        assert!(!app.settings.opaque_toolbit_path);
        click(&mut app, arrows[0].x, arrows[0].y);
        assert_eq!(app.settings.diameter, 8.1);
        click(&mut app, arrows[1].x, arrows[1].y);
        assert_eq!(app.settings.diameter, 8.);
        click(&mut app, arrows[2].x, arrows[2].y);
        assert_eq!(app.settings.speed, 9.);
        click(&mut app, arrows[3].x, arrows[3].y);
        assert_eq!(app.settings.speed, 8.);
        click(&mut app, 1187., 773.);
        assert_eq!(app.popup, 0);
        click(&mut app, 800., 750.);
        assert!(
            app.step > 0 && app.step + 1 < app.model.executions.len(),
            "Slider must seek"
        );
        let key = |key| {
            vec![egui::Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Modifiers::NONE,
            }]
        };
        let command_key = |key| {
            vec![egui::Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Modifiers::COMMAND,
            }]
        };
        frame(&mut app, key(Key::Home));
        assert_eq!(app.step, 0, "Focused slider must handle Home");
        frame(&mut app, key(Key::End));
        assert_eq!(app.step, max_step(&app.model));
        click(&mut app, 914., 775.);
        assert!(app.running, "Play must start");
        click(&mut app, 914., 775.);
        assert!(!app.running, "Same fixed cell must pause");
        click(&mut app, 110., 54.);
        assert!(app.settings.editor_open && !app.settings.code_open);
        click(&mut app, 202., 126.);
        assert!(!app.settings.machine_open);
        click(&mut app, 202., 126.);
        assert!(app.settings.machine_open);
        click(&mut app, 202., 90.);
        assert!(app.settings.dro_open);
        click(&mut app, 18., 51.);
        assert!(!app.settings.editor_open, "Gutter must hide the sidebar");
        click(&mut app, 18., 51.);
        assert!(app.settings.editor_open, "Plot tab must restore sidebar");
        assert!(!app.settings.code_open && app.settings.dro_open && app.settings.machine_open);
        frame(&mut app, vec![]);
        click(&mut app, 202., 90.);
        assert!(!app.settings.dro_open);
        click(&mut app, 330., 54.);
        assert!(app.settings.code_open);
        let movement = app
            .model
            .moves
            .iter()
            .find(|m| !m.rapid && m.seconds > 0.)
            .unwrap();
        app.time = movement.start_time + movement.seconds * 0.3;
        app.step = movement.step - 1;
        app.running = true;
        let before = app.position();
        app.running = false;
        assert_eq!(
            app.position(),
            before,
            "Pause must preserve fractional motion position"
        );
        app.model = Arc::new(
            inspect(
                "G0X0\nG1X10F600\nM0\nG1X20\nM30",
                "stops.nc",
                &Default::default(),
                &Default::default(),
            )
            .unwrap(),
        );
        app.seek(0);
        app.play(true);
        app.last_frame = Instant::now() - std::time::Duration::from_secs(1);
        app.advance();
        assert!(!app.running);
        assert_eq!(app.step, 2, "Timed forward play must stop at M0");
        app.play(true);
        app.last_frame = Instant::now() - std::time::Duration::from_secs(1);
        app.advance();
        assert_eq!(app.step, 4, "Resume must pass the same stop");
        app.play(false);
        app.last_frame = Instant::now() - std::time::Duration::from_secs(1);
        app.advance();
        assert!(!app.running);
        assert_eq!(app.step, 2, "Reverse play must honor the stop too");
        app.play(false);
        app.last_frame = Instant::now() - std::time::Duration::from_secs(1);
        app.advance();
        assert_eq!(app.step, 0, "Reverse must reach the first executed block");
        assert!(!app.running);

        let stopped = app.model.clone();
        app.model = Arc::new(
            inspect(
                "G0 X0\n#3006=1 (First)\n#3006=1 (Second)\nG1 X1 F60",
                "stops.nc",
                &Default::default(),
                &Default::default(),
            )
            .unwrap(),
        );
        app.seek(0);
        for step in [1, 2, 3] {
            app.play(true);
            app.last_frame = Instant::now() - std::time::Duration::from_secs(1);
            app.advance();
            assert_eq!(
                app.step, step,
                "Stops at the same time must resume separately"
            );
            assert!(!app.running);
        }
        app.model = Arc::new(
            inspect(
                "G0 X5\nG55 G4 X1\nM30",
                "dwell.nc",
                &Default::default(),
                &Options {
                    setup: serde_json::json!({"workOffsets":{"55":[2,0,0]}}),
                    ..Options::default()
                },
            )
            .unwrap(),
        );
        assert_eq!(app.model.status, "complete");
        app.step = 1;
        app.time = 0.5;
        assert_eq!(
            app.dro().0[0],
            5.,
            "Dwell retains the prior DRO until its block is reached"
        );
        app.seek(1);
        assert_eq!(
            app.dro().0[0],
            3.,
            "A manual seek shows the completed block"
        );
        app.model = stopped;
        app.settings.feed_timed = false;
        app.seek(0);
        app.play(true);
        app.last_frame = Instant::now() - std::time::Duration::from_millis(20);
        app.advance();
        assert_eq!(app.step, 0, "Untimed play must not depend on frame rate");
        app.last_frame = Instant::now() - std::time::Duration::from_millis(30);
        app.advance();
        assert_eq!(app.step, 1);

        let dir = std::env::temp_dir().join(format!("cnc-viewer-save-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let first = dir.join("first.nc");
        let second = dir.join("second.nc");
        std::fs::write(&first, "G0 X1\nM30").unwrap();
        std::fs::write(&second, "G0 X2\nM30").unwrap();
        app.open(vec![first.clone(), second.clone()], &ctx);
        let shapes = frame(&mut app, vec![]);
        let title_end = shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.job.text == "first.nc" => {
                    Some(text.pos.x + text.galley.size().x)
                }
                _ => None,
            })
            .unwrap();
        click(&mut app, title_end + 13., 54.);
        let shapes = frame(&mut app, vec![]);
        let source = shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.job.text == "second.nc" => {
                    Some(text.pos + text.galley.size() / 2.)
                }
                _ => None,
            })
            .unwrap();
        click(&mut app, source.x, source.y);
        assert_eq!(app.selected, "second.nc");
        assert!(
            app.settings.code_open,
            "Source picker must only switch files"
        );
        app.main = "second.nc".to_owned();
        app.documents
            .insert("first.nc".to_owned(), "G0 X3\nM30".into());
        app.dirty.insert("first.nc".to_owned());
        assert!(app.save_document("first.nc", false));
        assert!(!app.dirty.contains("first.nc"));
        let saved = std::fs::read_to_string(&first).unwrap();
        let other = std::fs::read_to_string(&second).unwrap();
        let now = Instant::now();
        std::fs::write(&second, "G0 X200\nM30").unwrap();
        app.watch_disk(&ctx, false, now);
        assert_eq!(app.documents["second.nc"], other);
        for _ in 0..80 {
            app.watch_disk(&ctx, true, now);
            if app.documents["second.nc"] != other {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(app.documents["second.nc"], "G0 X200\nM30");
        assert!(!app.dirty.contains("second.nc"));

        app.selected = "first.nc".into();
        app.documents
            .insert("first.nc".into(), "G0 X400\nM30".into());
        app.edited(&ctx);
        std::fs::write(&first, "G0 X500\nM30").unwrap();
        frame(&mut app, command_key(Key::S));
        assert!(app.overwrite.is_some());
        assert_eq!(std::fs::read_to_string(&first).unwrap(), "G0 X500\nM30");
        frame(&mut app, vec![]);
        frame(&mut app, key(Key::Enter));
        assert_eq!(std::fs::read_to_string(&first).unwrap(), "G0 X400\nM30");
        assert!(!app.dirty.contains("first.nc"));

        app.documents
            .insert("first.nc".into(), "G0 X600\nM30".into());
        app.edited(&ctx);
        std::fs::write(&first, "G0 X700\nM30").unwrap();
        app.refresh_key(&ctx, now);
        assert!(app.disk.files["first.nc"].out_of_sync());
        assert_eq!(app.documents["first.nc"], "G0 X600\nM30");
        app.settings.editor_open = true;
        frame(&mut app, vec![]);
        let apply = app.local_id("apply-sync");
        let reload = app.local_id(("sync-choice", "Reload"));
        let overwrite = app.local_id(("sync-choice", "Overwrite"));
        let disabled = ctx.read_response(apply).unwrap().rect.center();
        click(&mut app, disabled.x, disabled.y);
        assert_eq!(app.documents["first.nc"], "G0 X600\nM30");
        assert!(
            app.settings.code_open,
            "Disabled Apply must leave the editor open"
        );
        let reload_button = ctx.read_response(reload).unwrap().rect;
        assert!(
            reload_button.center().y < 72.,
            "Normal width keeps the controls by the filename"
        );
        click(
            &mut app,
            reload_button.right() - 8.,
            reload_button.center().y,
        );
        assert!(app
            .conflict_choice
            .as_ref()
            .is_some_and(|(_, choice)| *choice == disk::Choice::Reload));
        frame(&mut app, vec![]);
        let overwrite_button = ctx.read_response(overwrite).unwrap().rect;
        click(
            &mut app,
            overwrite_button.right() - 8.,
            overwrite_button.center().y,
        );
        assert!(app
            .conflict_choice
            .as_ref()
            .is_some_and(|(_, choice)| *choice == disk::Choice::Overwrite));
        click(
            &mut app,
            overwrite_button.right() - 8.,
            overwrite_button.center().y,
        );
        assert!(app.conflict_choice.is_none());
        app.settings.left_width = view::MIN_SIDEBAR_WIDTH;
        frame(&mut app, vec![]);
        frame(&mut app, vec![]);
        let narrow_apply = ctx.read_response(apply).unwrap();
        assert!(
            narrow_apply.rect.center().y > 72. && narrow_apply.rect.max.x < view::MIN_SIDEBAR_WIDTH,
            "{:?}",
            narrow_apply.rect
        );
        app.settings.left_width = 450.;
        frame(&mut app, vec![]);
        frame(&mut app, vec![]);
        click(
            &mut app,
            reload_button.right() - 8.,
            reload_button.center().y,
        );
        frame(&mut app, vec![]);
        click(&mut app, disabled.x, disabled.y);
        assert_eq!(app.documents["first.nc"], "G0 X700\nM30");
        app.documents
            .insert("first.nc".into(), "G0 X750\nM30".into());
        app.edited(&ctx);
        app.refresh_key(&ctx, now);
        assert_eq!(std::fs::read_to_string(&first).unwrap(), "G0 X700\nM30");
        app.refresh_key(&ctx, now + Duration::from_millis(100));
        assert_eq!(app.documents["first.nc"], "G0 X700\nM30");
        assert!(!app.disk.files["first.nc"].out_of_sync());
        app.update_title(&ctx);
        assert_eq!(
            app.title,
            format!("first.nc | {} | CNC Viewer", first.display())
        );
        app.documents
            .insert("first.nc".into(), "G0 X800\nM30".into());
        app.edited(&ctx);
        std::fs::write(&first, "G0 X900\nM30").unwrap();
        assert!(!app.save_document("first.nc", false));
        std::fs::write(&first, "G0 X1000\nM30").unwrap();
        assert!(
            !app.confirm_overwrite(),
            "Do not overwrite a version newer than the prompt"
        );
        assert_eq!(std::fs::read_to_string(&first).unwrap(), "G0 X1000\nM30");
        std::fs::remove_file(first).unwrap();
        std::fs::remove_file(second).unwrap();
        std::fs::remove_dir(dir).unwrap();
        assert_eq!(saved, "G0 X3\nM30");
        assert_eq!(other, "G0 X2\nM30");

        let source = "#100=0\nN20 #100=#100+1\nG1 X#100 F600\nIF [#100 LT 3] GOTO 20\nM30";
        let name = "loop.nc";
        app.model =
            Arc::new(inspect(source, name, &Default::default(), &Default::default()).unwrap());
        app.preview_stale = false;
        app.documents.insert(name.into(), source.into());
        app.settings.editor_open = true;
        app.seek(3);
        frame(&mut app, vec![]);
        ctx.memory_mut(|memory| memory.request_focus(app.editor_id()));
        frame(&mut app, key(Key::ArrowUp));
        assert_eq!(app.step, 5, "Editor must select the next loop visit");
        ctx.memory_mut(|memory| memory.request_focus(app.editor_id()));
        app.request_plot(false);
        let obsolete = app.cancel.as_ref().unwrap().clone();
        let active_model = app.model.clone();
        frame(&mut app, vec![egui::Event::Text("/".to_owned())]);
        assert_ne!(app.documents[name], source, "Editor must receive the edit");
        assert!(app.dirty.contains(name));
        assert!(obsolete.load(Ordering::Relaxed));
        assert!(app.preview_stale && Arc::ptr_eq(&active_model, &app.model));
        let first_deadline = app.edit_deadline.unwrap();
        app.schedule_refresh(&ctx, first_deadline - Duration::from_millis(50));
        let deadline = app.edit_deadline.unwrap();
        assert!(deadline > first_deadline);
        app.refresh_after_edit(&ctx, first_deadline, true);
        assert!(!app.busy);
        app.refresh_after_edit(&ctx, deadline, false);
        assert!(!app.busy);
        app.refresh_after_edit(&ctx, deadline, true);
        assert!(app.busy);
        click(&mut app, 19., 18.);
        assert_eq!(
            app.documents[name], source,
            "Toolbar Undo must restore the text"
        );
        click(&mut app, 51., 18.);
        assert_ne!(
            app.documents[name], source,
            "Toolbar Redo must restore the edit"
        );
        app.dirty.clear();
        app.apply_document_action(DocumentAction::Sample, &ctx);
        assert_eq!(app.main, "CNCViewerDemo.nc");
        assert!(app.documents[&app.main].contains("MACRO COMPASS"));
        let name = app.main.clone();
        app.settings.show_variables = false;
        frame(&mut app, command_key(Key::F));
        frame(&mut app, vec![egui::Event::Text("G0".into())]);
        assert!(app.find.open && !app.find.replace_open);
        let search_shapes = frame(&mut app, vec![]);
        for color in [
            Color32::from_rgb(255, 225, 154),
            Color32::from_rgb(255, 176, 93),
        ] {
            assert!(search_shapes.iter().any(|shape| {
                matches!(&shape.shape, egui::Shape::Vec(shapes) if shapes.iter().any(|shape| {
                    matches!(shape, egui::Shape::Rect(rect) if rect.fill == color)
                }))
            }));
        }
        let matches = app.find.results.as_ref().unwrap();
        let (first, count) = (matches.active, matches.ranges.len());
        assert!(count > 1);
        frame(&mut app, key(Key::Enter));
        assert_eq!(
            app.find.results.as_ref().unwrap().active,
            (first + 1) % count
        );
        frame(&mut app, key(Key::F3));
        assert_eq!(
            app.find.results.as_ref().unwrap().active,
            (first + 2) % count
        );
        assert!(ctx.memory(|memory| memory.has_focus(app.find_id())));
        frame(&mut app, command_key(Key::H));
        assert!(app.find.replace_open);
        frame(&mut app, vec![egui::Event::Text("WHILE".into())]);
        assert_eq!(app.find.query, "WHILE");
        assert_eq!(app.find.results.as_ref().unwrap().ranges.len(), 1);
        app.find.replacement = "while".into();
        click(&mut app, 370., 114.);
        assert!(app.documents[&name].contains("while"));
        frame(&mut app, key(Key::Escape));
        assert!(!app.find.open);
        ctx.memory_mut(|memory| memory.request_focus(app.editor_id()));
        frame(&mut app, command_key(Key::Z));
        assert!(app.documents[&name].contains("WHILE"));
        ctx.memory_mut(|memory| memory.request_focus(app.editor_id()));
        frame(&mut app, command_key(Key::Y));
        assert!(app.documents[&name].contains("while"));
        click(&mut app, 19., 18.);
        assert!(app.documents[&name].contains("WHILE"));
        click(&mut app, 51., 18.);
        assert!(app.documents[&name].contains("while"));
        ctx.memory_mut(|memory| memory.request_focus(app.editor_id()));
        frame(&mut app, command_key(Key::Home));
        frame(&mut app, key(Key::PageDown));
        let cursor = egui::text_edit::TextEditState::load(&ctx, app.editor_id())
            .unwrap()
            .cursor
            .char_range()
            .unwrap()
            .primary
            .index;
        assert!(usize::from(cursor) > 100);
        frame(&mut app, key(Key::PageUp));
        let earlier = egui::text_edit::TextEditState::load(&ctx, app.editor_id())
            .unwrap()
            .cursor
            .char_range()
            .unwrap()
            .primary
            .index;
        assert!(earlier < cursor);
        let scroll_to_end = vec![
            egui::Event::PointerMoved(egui::pos2(230., 240.)),
            egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: Vec2::new(0., -5000.),
                phase: egui::TouchPhase::Move,
                source: egui::MouseWheelSource::Trackpad,
                modifiers: Modifiers::NONE,
            },
        ];
        for _ in 0..3 {
            frame(&mut app, scroll_to_end.clone());
            let shapes = frame(&mut app, vec![]);
            let source = &app.documents[&name];
            let final_line = shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if &text.galley.job.text == source => {
                        let index =
                            text.galley.rows.len() - 1 - usize::from(source.ends_with('\n'));
                        Some(text.pos.y + text.galley.rows[index].min_y() - shape.clip_rect.min.y)
                    }
                    _ => None,
                })
                .unwrap();
            assert!(
                final_line.abs() <= 1.,
                "Last line at {final_line} pt from the top"
            );
        }
        if std::env::var_os("CNC_VIEWER_EDITOR_BENCH").is_some() {
            for rows in [100, 20_000, 100_000] {
                let source: String = (0..rows)
                    .map(|x| format!("G1 X{x}.25 Y30.125 Z-1.125 F600\n"))
                    .collect();
                app.documents.insert(name.clone(), source);
                app.highlight_cache.clear();
                frame(&mut app, vec![]);
                let mut samples = Vec::new();
                for _ in 0..4 {
                    ctx.memory_mut(|memory| memory.request_focus(app.editor_id()));
                    let length = app.documents[&name].len();
                    let start = Instant::now();
                    frame(&mut app, vec![egui::Event::Text(" ".to_owned())]);
                    samples.push(start.elapsed().as_secs_f64() * 1000.);
                    assert_eq!(app.documents[&name].len(), length + 1);
                }
                eprintln!("{rows} lines: {samples:?}");
            }
        }

        // Let the previous trackpad gesture stop before hovering in the new file.
        frame(&mut app, vec![egui::Event::PointerGone]);
        for _ in 0..45 {
            frame(&mut app, vec![]);
        }
        let bad_arc = "G90 G17 G21\nG0 X0 Y0\nG1 X2 Y#101 F100\nG2 X22 R4\nG1 X30\nM30\n";
        app.documents = BTreeMap::from([
            ("bad-arc.nc".into(), bad_arc.into()),
            ("O9012.nc".into(), "O9012\nG1 X\nM99\n".into()),
        ]);
        app.main = "bad-arc.nc".into();
        app.selected = app.main.clone();
        app.replot();
        finish_plot(&mut app);
        assert_eq!(app.model.status, "error");
        assert_eq!(app.diagnostics.len(), 3);
        let message = "Arc radius R is too small to reach the endpoint";
        let annotation = format!("·  {message}");
        let shapes = frame(&mut app, vec![]);
        assert!(shapes.iter().all(|shape| {
            !matches!(&shape.shape, egui::Shape::Text(text)
                if text.galley.job.text == "Diagnostics"
                    || (text.galley.job.text.contains(message) && text.galley.job.text != annotation))
        }));
        let row = shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.job.text == annotation => {
                    assert!(text.pos.x + text.galley.size().x > shape.clip_rect.max.x);
                    Some(text.pos.y + text.galley.size().y / 2.)
                }
                _ => None,
            })
            .expect("The arc line must have an inline diagnostic");
        assert!(shapes.iter().any(|shape| {
            matches!(&shape.shape, egui::Shape::Vec(shapes) if shapes.iter().any(|shape| {
                matches!(shape, egui::Shape::Rect(rect)
                    if rect.fill == Color32::from_rgb(255, 236, 240)
                        && rect.rect.contains(egui::pos2(100., row)))
            }))
        }));
        assert!(shapes.iter().any(|shape| {
            matches!(&shape.shape, egui::Shape::Vec(shapes) if shapes.iter().any(|shape| {
                matches!(shape, egui::Shape::Rect(rect) if rect.fill == Color32::from_rgb(255, 245, 220))
            }))
        }));
        let hovered = vec![egui::Event::PointerMoved(egui::pos2(100., row))];
        let has_hover_tip = |app: &mut App| {
            frame(app, hovered.clone());
            let mut shown = false;
            for _ in 0..45 {
                let shapes = frame(app, vec![]);
                shown |= shapes.iter().any(|shape| {
                    matches!(&shape.shape, egui::Shape::Text(text) if text.galley.job.text == message)
                });
            }
            shown
        };
        assert!(
            has_hover_tip(&mut app),
            "A clipped annotation must be readable on hover"
        );
        app.settings.left_width = 780.;
        frame(&mut app, vec![]);
        assert!(
            !has_hover_tip(&mut app),
            "A fully visible annotation must have no tooltip"
        );
        app.selected = "O9012.nc".into();
        assert!(frame(&mut app, vec![]).iter().any(|shape| {
            matches!(&shape.shape, egui::Shape::Text(text)
                if text.galley.job.text == "·  Address X is missing a value")
        }));

        let previous = app.model.clone();
        app.documents.insert(
            "O9012.nc".into(),
            "O9012\nN10 G0 X0\nN10 G0 X1\nM99\n".into(),
        );
        app.edited(&ctx);
        assert!(
            app.diagnostics.is_empty(),
            "Edits clear diagnostics from the previous text"
        );
        app.replot();
        finish_plot(&mut app);
        assert!(
            Arc::ptr_eq(&app.model, &previous),
            "A compile failure keeps the previous plot"
        );
        assert_eq!(app.diagnostics[0].source, "O9012.nc");
        assert_eq!(app.diagnostics[0].line, 3);
        assert_eq!(app.status, "Plot stopped");
        assert_eq!(
            app.diagnostics[0].error.message,
            "Duplicate sequence label N10"
        );

        for z_up in [true, false] {
            app.settings.z_up = z_up;
            app.camera.pitch = 0.12;
            let heading = app.camera.yaw;
            let start = egui::pos2(870., 400.);
            let press = |pos, pressed| egui::Event::PointerButton {
                pos,
                button: PointerButton::Secondary,
                pressed,
                modifiers: Modifiers::NONE,
            };
            frame(&mut app, vec![egui::Event::PointerMoved(start)]);
            frame(&mut app, vec![press(start, true)]);
            for (position, pitch) in [
                (egui::pos2(870., 390.), 0.04),
                (egui::pos2(1260., 380.), -0.04),
                (egui::pos2(1260., 370.), -0.12),
            ] {
                let shapes = frame(
                    &mut app,
                    vec![
                        egui::Event::PointerMoved(position),
                        egui::Event::MouseMoved(Vec2::new(0., -10.)),
                    ],
                );
                assert_eq!(app.camera.yaw, heading);
                assert!((app.camera.pitch - pitch).abs() < 1e-12);
                assert!((app.camera.orbit_direction()[2] - pitch.sin()).abs() < 1e-12);
                assert_eq!(
                    shapes.iter().any(|shape| matches!(&shape.shape,
                        egui::Shape::Text(text) if text.galley.job.text == "Underside")),
                    pitch < 0.,
                );
            }
            frame(&mut app, vec![press(egui::pos2(1260., 370.), false)]);
        }

        let name = "large-playback.tap";
        let mut source = "(A comment outside the playback viewport)\n".repeat(3500);
        source.push_str("G0 X0\nG1 X44 F100\nM30\n");
        app.documents.insert(name.into(), source.clone());
        app.main = name.into();
        app.selected = name.into();
        app.find = Default::default();
        app.diagnostics.clear();
        app.preview_stale = false;
        app.model =
            Arc::new(inspect(&source, name, &Default::default(), &Default::default()).unwrap());
        app.seek(3501);
        app.play(true);
        let shapes = frame(&mut app, vec![]);
        let snippet = shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(shape) if shape.galley.job.text.contains("G1 X44 F100") => {
                    Some(shape)
                }
                _ => None,
            })
            .expect("Playback must display the active code");
        let visible = &snippet.galley.job.text;
        assert!(visible.len() < 5000, "Playback shaped offscreen code");
        let row = visible[..visible.find("G1 X44 F100").unwrap()]
            .matches('\n')
            .count();
        let code = snippet.pos + Vec2::new(25., snippet.galley.rows[row].rect().center().y);
        click(&mut app, code.x, code.y);
        assert!(
            !app.running,
            "Clicking a running line must select its execution"
        );
        assert_eq!(app.selected_line, 3502);

        let editor = app.editor_id();
        app.play(true);
        ctx.memory_mut(|memory| memory.request_focus(editor));
        frame(&mut app, vec![egui::Event::Text("(edited)".into())]);
        assert!(!app.running);
        assert!(app.documents[name].contains("(edited)"));

        let mut recovered = project::Project::new(
            "recovered".into(),
            "recovered.nc",
            "G0 X0\nG1 X10 F100\nM30\n",
        );
        recovered.plot_paused = true;
        app.install_project(recovered);
        assert!(!app.busy && app.model.moves.is_empty());
        assert!(app.status.contains("Ctrl+Enter"));
        frame(&mut app, command_key(Key::Enter));
        finish_plot(&mut app);
        assert_eq!(app.model.moves.last().unwrap().end, [10., 0., 0.]);
    }
}

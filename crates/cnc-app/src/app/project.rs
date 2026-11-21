use super::{disk, find, parse_machine_setup, read_document, session, App, Camera, Settings};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::Arc,
};

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Project {
    pub id: String,
    pub opened: u64,
    pub edited: u64,
    pub documents: BTreeMap<String, String>,
    pub main: String,
    selected: String,
    pub files: BTreeMap<String, disk::File>,
    pub dirty: BTreeSet<String>,
    pub setup: Value,
    camera: Camera,
    resume: Option<PlotPosition>,
    settings: Option<Settings>,
    #[serde(rename = "plot_cleared")]
    pub(super) plot_paused: bool,
    find: find::Find,
    #[serde(skip)]
    pub(super) cached_plot: Option<CachedPlot>,
}

#[derive(Clone)]
pub(super) struct CachedPlot {
    model: Arc<super::Model>,
    geometry: super::PreparedPlot,
    diagnostics: Vec<super::Diagnostic>,
    status: String,
}

#[derive(Clone, Copy, Serialize, Deserialize)]
pub(super) struct PlotPosition {
    pub step: usize,
    pub time: f64,
    pub line: usize,
}

pub(super) const UNTITLED: &str = "New GCode File.nc";
pub(super) const SAMPLE: &str = "CNCViewerDemo.nc";
pub(super) const STARTER: &str = "(CNC Viewer)\n(Open a GCode file or choose Sample to begin.)\n\nG21 G90 G17\nG0 X0 Y0 Z5\nG1 Z0 F200\nX30\nY20\nX0\nY0\nG0 Z5\nM30\n";

impl Project {
    pub fn new(id: String, name: &str, text: &str) -> Self {
        Self {
            id,
            opened: session::now(),
            edited: 0,
            documents: BTreeMap::from([(name.into(), text.into())]),
            main: name.into(),
            selected: name.into(),
            files: BTreeMap::new(),
            dirty: BTreeSet::new(),
            setup: json!({}),
            camera: Camera::default(),
            resume: None,
            settings: None,
            plot_paused: false,
            find: find::Find::default(),
            cached_plot: None,
        }
    }

    fn open_document(id: String, path: &Path) -> Result<Self, Box<dyn std::error::Error>> {
        let (name, text, file) = read_document(path)?;
        let mut project = Self::new(id, &name, "");
        project.documents.insert(name.clone(), text);
        project.files.insert(name, file);
        Ok(project)
    }

    pub fn open(id: String, path: &Path) -> Result<Self, Box<dyn std::error::Error>> {
        let mut project = Self::open_document(id, path)?;
        let mut setup = path.as_os_str().to_os_string();
        setup.push(".setup.json");
        let setup = PathBuf::from(setup);
        match std::fs::read_to_string(&setup) {
            Ok(text) => {
                project.setup = parse_machine_setup(&text)
                    .map_err(|error| format!("{}: {error}", setup.display()))?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(format!("{}: {error}", setup.display()).into()),
        }
        Ok(project)
    }

    pub fn restore(tab: &session::OpenTab) -> Result<Self, Box<dyn std::error::Error>> {
        let mut project = Self::open_document(tab.id.clone(), &tab.path)?;
        project.opened = tab.opened;
        project.edited = tab.edited;
        project.setup.clone_from(&tab.setup);
        for path in &tab.libraries {
            if let Ok((name, text, file)) = read_document(path) {
                if !project.documents.contains_key(&name) {
                    project.documents.insert(name.clone(), text);
                    project.files.insert(name, file);
                }
            }
        }
        Ok(project)
    }

    pub fn path(&self) -> Option<&Path> {
        self.files.get(&self.main).map(|file| file.path.as_path())
    }

    pub fn label(&self) -> &str {
        file_label(&self.main, self.path())
    }

    pub fn record(&self) -> Option<session::OpenTab> {
        tab_record(
            &self.id,
            &self.main,
            &self.files,
            &self.setup,
            self.opened,
            self.edited,
        )
    }
}

pub(super) fn file_label<'a>(name: &'a str, path: Option<&'a Path>) -> &'a str {
    path.and_then(Path::file_name)
        .and_then(|name| name.to_str())
        .unwrap_or(name)
}

pub(super) fn tab_record(
    id: &str,
    main: &str,
    files: &BTreeMap<String, disk::File>,
    setup: &Value,
    opened: u64,
    edited: u64,
) -> Option<session::OpenTab> {
    let main_file = files.get(main)?;
    Some(session::OpenTab {
        id: id.into(),
        path: main_file.path.clone(),
        opened,
        edited,
        libraries: files
            .iter()
            .filter(|(name, _)| name.as_str() != main)
            .map(|(_, file)| file.path.clone())
            .collect::<Vec<PathBuf>>(),
        setup: setup.clone(),
    })
}

impl App {
    pub(super) fn snapshot_project(&self) -> Project {
        Project {
            id: self.project_id.clone(),
            opened: self.opened_at,
            edited: self.edited_at,
            documents: self.documents.clone(),
            main: self.main.clone(),
            selected: self.selected.clone(),
            files: self.disk.files.clone(),
            dirty: self.dirty.clone(),
            setup: self.program_setup.clone(),
            camera: self.camera.clone(),
            resume: self.project_position(),
            settings: Some(self.settings.clone()),
            plot_paused: self.plot_paused,
            find: self.find.clone(),
            cached_plot: None,
        }
    }

    fn project_position(&self) -> Option<PlotPosition> {
        self.resume_plot.or_else(|| {
            (!self.model.executions.is_empty()).then_some(PlotPosition {
                step: self.step,
                time: self.time,
                line: self.selected_line,
            })
        })
    }

    pub(super) fn take_project(&mut self) -> Project {
        let cached_plot = (self.plotted_generation == Some(self.generation)).then(|| CachedPlot {
            model: self.model.clone(),
            geometry: self.plot.prepared(),
            diagnostics: std::mem::take(&mut self.diagnostics),
            status: std::mem::take(&mut self.status),
        });
        self.cancel_plot();
        self.highlight_cache.clear();
        self.source_rows.clear();
        self.find.results = None;
        Project {
            id: std::mem::take(&mut self.project_id),
            opened: self.opened_at,
            edited: self.edited_at,
            documents: std::mem::take(&mut self.documents),
            main: std::mem::take(&mut self.main),
            selected: std::mem::take(&mut self.selected),
            files: std::mem::take(&mut self.disk.files),
            dirty: std::mem::take(&mut self.dirty),
            setup: std::mem::take(&mut self.program_setup),
            camera: self.camera.clone(),
            resume: self.project_position(),
            settings: Some(self.settings.clone()),
            plot_paused: self.plot_paused,
            find: std::mem::take(&mut self.find),
            cached_plot,
        }
    }

    pub(super) fn install_project(&mut self, project: Project) {
        self.cancel_plot();
        self.edit_deadline = None;
        self.project_id = project.id;
        self.opened_at = project.opened;
        self.edited_at = project.edited;
        self.documents = project.documents;
        self.main = project.main;
        self.selected = project.selected;
        self.disk.files = project.files;
        self.dirty = project.dirty;
        self.program_setup = project.setup;
        self.find = project.find;
        self.camera = project.camera;
        self.resume_plot = project.resume;
        if let Some(settings) = project.settings {
            self.settings = settings;
        }
        self.reset_disk_choices();
        if self.capture_path.is_none() {
            self.popup = 0;
        }
        self.title.clear();
        self.step = 0;
        self.time = 0.;
        self.selected_line = 0;
        self.scroll_line = None;
        self.last_stop = None;
        self.preview_stale = false;
        self.plot_paused = project.plot_paused;
        if let Some(cached) = project.cached_plot {
            self.model = cached.model;
            self.plot.install(cached.geometry);
            self.diagnostics = cached.diagnostics;
            self.status = cached.status;
            self.plotted_generation = Some(self.generation);
            if let Some(resume) = self.resume_plot.take() {
                self.restore_position(resume);
            }
            return;
        }
        self.model = Arc::default();
        self.plot.clear();
        if self.plot_paused {
            self.status = "Plot paused. Press Ctrl+Enter to plot.".into();
            self.diagnostics.clear();
        } else {
            self.request_plot(self.resume_plot.is_none());
        }
    }
}

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::BTreeSet,
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

pub(super) const RESTORE_LIMIT: usize = 7;

pub(super) fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros() as u64
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(super) struct OpenTab {
    pub id: String,
    pub path: PathBuf,
    pub opened: u64,
    pub edited: u64,
    pub libraries: Vec<PathBuf>,
    pub setup: Value,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub(super) struct Recent {
    pub path: PathBuf,
    pub opened: u64,
}

#[derive(Default, Serialize, Deserialize)]
struct Log {
    tabs: Vec<OpenTab>,
    recent: Vec<Recent>,
}

pub(super) struct Session {
    root: PathBuf,
    lease: Option<(PathBuf, File)>,
}

impl Session {
    pub fn start(root: PathBuf, restore: bool) -> io::Result<(Self, Vec<OpenTab>, bool)> {
        let windows = root.join("windows");
        fs::create_dir_all(&windows)?;
        let mut session = Self { root, lease: None };
        let _lock = session.lock()?;
        let log = session.read()?;
        let mut another_window = false;
        let mut interrupted = false;
        for entry in fs::read_dir(&windows)? {
            let path = entry?.path();
            if path.extension().and_then(|extension| extension.to_str()) != Some("lock") {
                continue;
            }
            let Ok(file) = OpenOptions::new().read(true).write(true).open(&path) else {
                continue;
            };
            if file.try_lock().is_ok() {
                interrupted = true;
                drop(file);
                let _ = fs::remove_file(path);
            } else {
                another_window = true;
            }
        }
        let path = windows.join(format!("{}-{}.lock", std::process::id(), now()));
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)?;
        file.lock()?;
        session.lease = Some((path, file));
        let candidates = if restore && !another_window {
            let mut tabs = log.tabs;
            tabs.sort_by(|a, b| {
                (b.opened.max(b.edited), &b.id).cmp(&(a.opened.max(a.edited), &a.id))
            });
            tabs
        } else {
            Vec::new()
        };
        Ok((session, candidates, interrupted))
    }

    fn lock(&self) -> io::Result<File> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(self.root.join("session.lock"))?;
        file.lock()?;
        Ok(file)
    }

    fn read(&self) -> io::Result<Log> {
        match fs::read(self.root.join("session.json")) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(io::Error::other),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Log::default()),
            Err(error) => Err(error),
        }
    }

    fn update(&self, edit: impl FnOnce(&mut Log)) -> io::Result<()> {
        let _lock = self.lock()?;
        let mut log = self.read()?;
        edit(&mut log);
        let temporary = self.root.join("session.tmp");
        let mut file = File::create(&temporary)?;
        serde_json::to_writer(&mut file, &log)?;
        file.flush()?;
        file.sync_all()?;
        drop(file);
        fs::rename(temporary, self.root.join("session.json"))
    }

    pub fn record(&self, tab: OpenTab) -> io::Result<()> {
        self.update(|log| {
            let mut opened = true;
            if let Some(existing) = log.tabs.iter_mut().find(|open| open.id == tab.id) {
                opened = existing.path != tab.path || existing.opened != tab.opened;
                existing.clone_from(&tab);
            } else {
                log.tabs.push(tab.clone());
            }
            if !opened {
                return;
            }
            if let Some(recent) = log.recent.iter_mut().find(|recent| recent.path == tab.path) {
                recent.opened = recent.opened.max(tab.opened);
            } else {
                log.recent.push(Recent {
                    path: tab.path,
                    opened: tab.opened,
                });
            }
            log.recent
                .sort_by_key(|recent| std::cmp::Reverse(recent.opened));
        })
    }

    pub fn forget(&self, ids: &BTreeSet<&str>) -> io::Result<()> {
        self.update(|log| log.tabs.retain(|tab| !ids.contains(tab.id.as_str())))
    }

    pub fn recent(&self) -> io::Result<Vec<Recent>> {
        // Writers publish the complete log with a rename.
        Ok(self.read()?.recent)
    }

    pub fn history_store(&self) -> Self {
        Self {
            root: self.root.clone(),
            lease: None,
        }
    }

    pub fn prune_recent(&self, missing: &BTreeSet<Recent>) -> io::Result<()> {
        self.update(|log| log.recent.retain(|recent| !missing.contains(recent)))
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        if let Some((path, file)) = self.lease.take() {
            let lock = self.lock();
            drop(file);
            let _ = fs::remove_file(path);
            drop(lock);
        }
    }
}

pub(super) fn canonical(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_owned())
}

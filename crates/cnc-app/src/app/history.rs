use super::session::{Recent, Session};
use eframe::egui;
use std::{
    collections::BTreeSet,
    fs,
    io::ErrorKind,
    path::{Component, Path, PathBuf},
    sync::mpsc,
};

pub(super) struct History {
    pub rows: Vec<Recent>,
    pub missing: BTreeSet<Recent>,
    local: Option<mpsc::SyncSender<Vec<Recent>>>,
    network: Option<mpsc::SyncSender<Vec<Recent>>>,
    results: (
        mpsc::Sender<BTreeSet<Recent>>,
        mpsc::Receiver<BTreeSet<Recent>>,
    ),
}

impl History {
    pub fn new() -> Self {
        Self {
            rows: Vec::new(),
            missing: BTreeSet::new(),
            local: None,
            network: None,
            results: mpsc::channel(),
        }
    }

    pub fn receive(&mut self) {
        for missing in self.results.1.try_iter() {
            self.missing.extend(missing);
        }
    }

    pub fn open(&mut self, session: Option<&Session>, ctx: &egui::Context) {
        self.receive();
        if let Some(session) = session {
            if let Ok(rows) = session.recent() {
                self.rows = rows;
            }
            self.rows.retain(|entry| !self.missing.contains(entry));
            self.missing.clear();
            let (network, local): (Vec<_>, Vec<_>) = self
                .rows
                .iter()
                .cloned()
                .partition(|entry| is_unc(&entry.path));
            for (worker, entries, batch_size) in [
                (&mut self.local, local, 32),
                (&mut self.network, network, 1),
            ] {
                if !entries.is_empty() {
                    let worker = worker.get_or_insert_with(|| {
                        scan_queue(
                            session.history_store(),
                            self.results.0.clone(),
                            ctx.clone(),
                            batch_size,
                        )
                    });
                    let _ = worker.try_send(entries);
                }
            }
        }
    }
}

fn scan_queue(
    store: Session,
    results: mpsc::Sender<BTreeSet<Recent>>,
    ctx: egui::Context,
    batch_size: usize,
) -> mpsc::SyncSender<Vec<Recent>> {
    // A slow share can occupy its worker without holding up local files or the UI.
    let (queue, jobs) = mpsc::sync_channel::<Vec<Recent>>(1);
    std::thread::spawn(move || {
        while let Ok(entries) = jobs.recv() {
            for batch in entries.chunks(batch_size) {
                let missing: BTreeSet<_> = batch
                    .iter()
                    .filter(|entry| confirmed_missing(&entry.path))
                    .cloned()
                    .collect();
                if !missing.is_empty() {
                    let _ = store.prune_recent(&missing);
                    if results.send(missing).is_err() {
                        return;
                    }
                    ctx.request_repaint();
                }
            }
        }
    });
    queue
}

#[cfg(windows)]
fn is_unc(path: &Path) -> bool {
    use std::path::Prefix;
    matches!(
        path.components().next(),
        Some(Component::Prefix(prefix))
            if matches!(prefix.kind(), Prefix::UNC(..) | Prefix::VerbatimUNC(..))
    )
}

#[cfg(not(windows))]
fn is_unc(_: &Path) -> bool {
    false
}

fn confirmed_missing(path: &Path) -> bool {
    match fs::metadata(path) {
        Ok(metadata) => !metadata.is_file(),
        Err(error) if error.kind() == ErrorKind::NotFound => {
            // A disconnected drive or share can also produce NotFound.
            #[cfg(windows)]
            if !matches!(error.raw_os_error(), Some(2 | 3)) {
                return false;
            }
            let root: PathBuf = path
                .components()
                .take_while(|part| matches!(part, Component::Prefix(_) | Component::RootDir))
                .collect();
            root.has_root()
                && (fs::metadata(&root).is_ok_and(|m| m.is_dir())
                    || is_unc(path) && fs::read_dir(&root).is_ok())
        }
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::session::{self, OpenTab};
    use std::time::Duration;

    #[test]
    fn scan_keeps_rows_in_place_and_prunes_only_the_checked_history_entry() {
        let root = std::env::temp_dir().join(format!(
            "cnc-history-{}-{}",
            std::process::id(),
            session::now()
        ));
        let (store, _) = Session::start(root.clone(), false).unwrap();
        let present = root.join("present.nc");
        let missing = root.join("removed-folder").join("missing.nc");
        fs::write(&present, "M30\n").unwrap();
        let tab = |path: &Path, opened| OpenTab {
            id: path.display().to_string(),
            path: path.to_owned(),
            opened,
            edited: opened,
            libraries: Vec::new(),
            setup: serde_json::json!({}),
        };
        store.record(tab(&present, 1)).unwrap();
        store.record(tab(&missing, 2)).unwrap();
        let ctx = egui::Context::default();
        let mut history = History::new();
        history.open(Some(&store), &ctx);
        assert_eq!(history.rows[0].path, missing);
        assert_eq!(history.rows[1].path, present);
        assert!(history.missing.is_empty());

        let result = history
            .results
            .1
            .recv_timeout(Duration::from_secs(3))
            .unwrap();
        history.results.0.send(result.clone()).unwrap();
        history.receive();
        assert_eq!(history.rows[0].path, missing);
        assert!(history.missing.contains(&history.rows[0]));
        assert!(!history.missing.contains(&history.rows[1]));
        assert_eq!(store.recent().unwrap().as_slice(), &history.rows[1..]);

        let mut edited = tab(&missing, 2);
        edited.edited = 3;
        store.record(edited).unwrap();
        history.open(Some(&store), &ctx);
        assert_eq!(history.rows.len(), 1);
        assert_eq!(history.rows[0].path, present);
        drop(history);
        fs::create_dir(root.join("removed-folder")).unwrap();
        fs::write(&missing, "M30\n").unwrap();
        store.record(tab(&missing, 4)).unwrap();
        store.prune_recent(&result).unwrap();
        assert_eq!(store.recent().unwrap()[0].opened, 4);
        assert_eq!(store.recent().unwrap().len(), 2);
        drop(store);
        fs::remove_file(missing).unwrap();
        fs::remove_file(present).unwrap();
        fs::remove_file(root.join("session.json")).unwrap();
        fs::remove_file(root.join("session.lock")).unwrap();
        fs::remove_dir(root.join("removed-folder")).unwrap();
        fs::remove_dir(root.join("windows")).unwrap();
        fs::remove_dir(root).unwrap();
    }
}

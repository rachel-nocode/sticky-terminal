use std::collections::VecDeque;
use std::path::PathBuf;

#[derive(Clone, PartialEq)]
pub(crate) enum ChangeKind {
    Created,
    Modified,
    Deleted,
}

#[derive(Clone)]
pub(crate) struct FileChange {
    pub(crate) path: PathBuf,
    pub(crate) kind: ChangeKind,
    pub(crate) when: std::time::Instant,
}

pub(crate) struct FileWatcher {
    pub(crate) rx: std::sync::mpsc::Receiver<notify_debouncer_mini::DebounceEventResult>,
    pub(crate) _debouncer: notify_debouncer_mini::Debouncer<notify::RecommendedWatcher>,
    pub(crate) watched_path: PathBuf,
    pub(crate) recent_changes: VecDeque<FileChange>,
}

impl FileWatcher {
    pub(crate) fn start(path: PathBuf) -> anyhow::Result<Self> {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut debouncer = notify_debouncer_mini::new_debouncer(
            std::time::Duration::from_millis(300),
            move |res| { let _ = tx.send(res); },
        )?;
        debouncer.watcher().watch(&path, notify::RecursiveMode::Recursive)?;
        Ok(Self {
            rx,
            _debouncer: debouncer,
            watched_path: path,
            recent_changes: VecDeque::new(),
        })
    }

    pub(crate) fn drain(&mut self) -> bool {
        let mut got = false;
        while let Ok(result) = self.rx.try_recv() {
            if let Ok(events) = result {
                for event in events {
                    let skip = event.path.components().any(|c| c.as_os_str() == ".git");
                    if skip { continue; }
                    // debouncer-mini only has Any/AnyContinuous — treat all as Modified
                    let kind = ChangeKind::Modified;

                    let existing = self.recent_changes.iter_mut()
                        .find(|c| c.path == event.path && c.kind == kind);
                    if let Some(c) = existing {
                        c.when = std::time::Instant::now();
                    } else {
                        self.recent_changes.push_front(FileChange {
                            path: event.path.clone(),
                            kind,
                            when: std::time::Instant::now(),
                        });
                        if self.recent_changes.len() > 20 {
                            self.recent_changes.pop_back();
                        }
                    }
                    got = true;
                }
            }
        }
        got
    }
}

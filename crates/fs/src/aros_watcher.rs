//! Native file watching for AROS, over the host's kqueue.
//!
//! AROS cannot report file changes, but the host underneath it can. Each
//! `watch()` becomes one host-side directory watch (`aros_fsw_*` in the
//! editor's C glue, over `hostlib.resource`), and a polling thread drains the
//! shared kqueue every 250 ms -- so a change arrives in a quarter of a second
//! where the tree-walking `PollWatcher` needed 20 to 30.
//!
//! Shape: this implements `notify::Watcher` the way inotify does -- one
//! NON-recursive watch per directory, the caller (the worktree scanner)
//! registering each directory itself and reacting to events by rescanning that
//! one directory. AROS already takes that code path, being neither macOS nor
//! Windows, so nothing above this file changes.
//!
//! Paths: the caller watches AROS paths (`MacRW:proj/src`), the host watches
//! host paths. The mapping comes from `AROS_FSW_ROOTS`
//! (`MacRW:=/Users/x/Shared;SYS:=/Users/x/aros/AROS`), set by the boot
//! harness. A path with no mapped volume cannot be watched natively and
//! reports itself as such (`requires_kqueue_fallback`), which routes it to the
//! poll watcher upfront.

use notify::{Config, Event, EventHandler, EventKind, RecursiveMode, Watcher, WatcherKind};
use parking_lot::Mutex;
use std::collections::HashMap;
use std::ffi::CString;
use std::os::raw::{c_char, c_int, c_uint};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

unsafe extern "C" {
    fn aros_fsw_init() -> c_int;
    fn aros_fsw_add(host_path: *const c_char) -> c_int;
    fn aros_fsw_remove(id: c_int);
    fn aros_fsw_poll(ids: *mut c_int, flags: *mut c_uint, cap: c_int) -> c_int;
}

const NOTE_DELETE: u32 = 0x1;
const NOTE_RENAME: u32 = 0x20;

/// More fds than this and we stop adding watches; the registration machinery
/// treats the resulting `MaxFilesWatch` as "fall back to polling for this
/// path", which is exactly right.
const MAX_WATCHES: usize = 1024;

/// `VOLUME:` -> host root, parsed once from `AROS_FSW_ROOTS`.
fn volume_map() -> &'static Vec<(String, PathBuf)> {
    static MAP: OnceLock<Vec<(String, PathBuf)>> = OnceLock::new();
    MAP.get_or_init(|| {
        let Ok(spec) = std::env::var("AROS_FSW_ROOTS") else {
            return Vec::new();
        };
        spec.split(';')
            .filter_map(|entry| {
                let (vol, root) = entry.split_once('=')?;
                let vol = vol.trim();
                let root = root.trim();
                (!vol.is_empty() && vol.ends_with(':') && !root.is_empty())
                    .then(|| (vol.to_string(), PathBuf::from(root)))
            })
            .collect()
    })
}

/// The host path for an AROS path, when its volume is mapped.
fn to_host_path(path: &Path) -> Option<PathBuf> {
    let s = path.to_str()?;
    for (vol, root) in volume_map() {
        if let Some(rest) = s.strip_prefix(vol.as_str()) {
            let rest = rest.trim_start_matches('/');
            return Some(if rest.is_empty() { root.clone() } else { root.join(rest) });
        }
    }
    None
}

/// True when this path cannot be watched through the host kqueue and should go
/// to the poll watcher instead.
pub fn requires_kqueue_fallback(path: &Path) -> bool {
    to_host_path(path).is_none()
}

struct WatchTable {
    by_id: HashMap<c_int, Arc<Path>>,
    by_path: HashMap<Arc<Path>, c_int>,
}

struct Shared {
    table: Mutex<WatchTable>,
    handler: Mutex<Box<dyn EventHandler>>,
}

pub struct ArosKqueueWatcher {
    shared: Arc<Shared>,
}

impl ArosKqueueWatcher {
    fn build<H: EventHandler>(handler: H) -> notify::Result<Self> {
        if unsafe { aros_fsw_init() } != 0 {
            return Err(notify::Error::generic(
                "host kqueue unavailable (no hostlib, or not a darwin host)",
            ));
        }
        let shared = Arc::new(Shared {
            table: Mutex::new(WatchTable { by_id: HashMap::new(), by_path: HashMap::new() }),
            handler: Mutex::new(Box::new(handler)),
        });
        let for_thread = Arc::downgrade(&shared);
        std::thread::Builder::new()
            .name("aros-kqueue-watch".into())
            .spawn(move || {
                let mut ids = [0 as c_int; 32];
                let mut flags = [0 as c_uint; 32];
                loop {
                    let Some(shared) = for_thread.upgrade() else { break };
                    let n = unsafe {
                        aros_fsw_poll(ids.as_mut_ptr(), flags.as_mut_ptr(), ids.len() as c_int)
                    };
                    for i in 0..n.max(0) as usize {
                        shared.dispatch(ids[i], flags[i]);
                    }
                    drop(shared);
                    std::thread::sleep(Duration::from_millis(250));
                }
            })
            .map_err(|e| notify::Error::generic(&format!("cannot spawn watch thread: {e}")))?;
        Ok(Self { shared })
    }
}

impl Shared {
    fn dispatch(&self, id: c_int, fflags: u32) {
        let gone = fflags & (NOTE_DELETE | NOTE_RENAME) != 0;
        let path = {
            let mut table = self.table.lock();
            let Some(path) = table.by_id.get(&id).cloned() else { return };
            if gone {
                // The fd tracks the old object; the name now needs a fresh
                // watch if something reappears there. Re-adding is the
                // caller's reaction to the event (it rescans and re-watches),
                // so just retire this one.
                table.by_id.remove(&id);
                table.by_path.remove(&path);
                unsafe { aros_fsw_remove(id) };
            }
            path
        };
        let kind =
            if gone { EventKind::Remove(notify::event::RemoveKind::Any) } else { EventKind::Modify(notify::event::ModifyKind::Any) };
        self.handler
            .lock()
            .handle_event(Ok(Event { kind, paths: vec![path.to_path_buf()], attrs: Default::default() }));
    }
}

impl Watcher for ArosKqueueWatcher {
    fn new<H: EventHandler>(handler: H, _config: Config) -> notify::Result<Self> {
        Self::build(handler)
    }

    fn watch(&mut self, path: &Path, _mode: RecursiveMode) -> notify::Result<()> {
        let host = to_host_path(path).ok_or_else(|| {
            notify::Error::generic(&format!("no host mapping for {}", path.display()))
                .add_path(path.to_path_buf())
        })?;
        let mut table = self.shared.table.lock();
        let key: Arc<Path> = Arc::from(path);
        if table.by_path.contains_key(&key) {
            return Ok(());
        }
        if table.by_id.len() >= MAX_WATCHES {
            return Err(notify::Error {
                kind: notify::ErrorKind::MaxFilesWatch,
                paths: vec![path.to_path_buf()],
            });
        }
        let c = CString::new(host.to_str().unwrap_or_default())
            .map_err(|_| notify::Error::generic("path with NUL"))?;
        let id = unsafe { aros_fsw_add(c.as_ptr()) };
        if id < 0 {
            return Err(notify::Error::generic(&format!(
                "host watch failed for {} ({})",
                path.display(),
                host.display()
            ))
            .add_path(path.to_path_buf()));
        }
        table.by_id.insert(id, key.clone());
        table.by_path.insert(key, id);
        Ok(())
    }

    fn unwatch(&mut self, path: &Path) -> notify::Result<()> {
        let mut table = self.shared.table.lock();
        let key: Arc<Path> = Arc::from(path);
        if let Some(id) = table.by_path.remove(&key) {
            table.by_id.remove(&id);
            unsafe { aros_fsw_remove(id) };
        }
        Ok(())
    }

    fn kind() -> WatcherKind
    where
        Self: Sized,
    {
        WatcherKind::Kqueue
    }
}

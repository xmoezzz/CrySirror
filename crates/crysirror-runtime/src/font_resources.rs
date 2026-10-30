//! Bounded I/O -> CPU -> publication font jobs, with client-owned immutable faces.
use anyhow::{Context, Result, ensure};
use crysirror_core::{service::*, state::ClientId};
use crysirror_tasks::{Task, TaskPermit, TaskPool};
use crysirror_text::{
    TextSystem,
    dynamic::{MAX_FONT_BYTES, PreparedFontFace},
};
use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};
const MAX_RESIDENT: usize = 256 * 1024 * 1024;
struct Charge {
    used: Arc<AtomicUsize>,
    bytes: usize,
}
impl Drop for Charge {
    fn drop(&mut self) {
        self.used.fetch_sub(self.bytes, Ordering::AcqRel);
    }
}
fn charge(used: &Arc<AtomicUsize>, len: usize) -> Result<Arc<Charge>> {
    ensure!(len > 0 && len <= MAX_FONT_BYTES, "font source size limit");
    let bytes = len.checked_mul(3).context("font charge overflow")?;
    used.fetch_update(Ordering::AcqRel, Ordering::Acquire, |old| {
        old.checked_add(bytes).filter(|n| *n <= MAX_RESIDENT)
    })
    .map_err(|_| anyhow::anyhow!("font resident byte budget"))?;
    Ok(Arc::new(Charge {
        used: used.clone(),
        bytes,
    }))
}
struct Loaded {
    bytes: Arc<[u8]>,
    charge: Arc<Charge>,
}
enum Stage {
    Read(Task<Loaded>, Option<TaskPermit>),
    Prepare(Task<Arc<PreparedFontFace>>),
    Done(FontLoadState),
}
struct Job {
    owner: ClientId,
    name: String,
    index: u32,
    stage: Stage,
}
struct State {
    next: u64,
    jobs: BTreeMap<u64, Job>,
    fonts: BTreeMap<u64, FontInfo>,
}
struct Shared {
    text: TextSystem,
    io: TaskPool,
    cpu: TaskPool,
    used: Arc<AtomicUsize>,
    state: Mutex<State>,
}
/// Shared font resource owner. Clones refer to one bounded catalog and job queue.
#[derive(Clone)]
pub struct FontResources(Arc<Shared>);
fn err(e: impl std::fmt::Display) -> crysirror_core::Error {
    crysirror_core::Error::Io(e.to_string())
}
impl FontResources {
    pub fn new(text: TextSystem, io: TaskPool, cpu: TaskPool) -> Self {
        Self(Arc::new(Shared {
            text,
            io,
            cpu,
            used: Arc::new(AtomicUsize::new(0)),
            state: Mutex::new(State {
                next: 1,
                jobs: BTreeMap::new(),
                fonts: BTreeMap::new(),
            }),
        }))
    }
    pub fn for_client(
        &self,
        owner: ClientId,
        storage: Arc<dyn ClientStorage>,
    ) -> Arc<dyn FontLibrary> {
        Arc::new(ClientFonts {
            resources: self.clone(),
            owner,
            storage,
        })
    }
    /// Whether a snapshot would need explicit dynamic-font reconstruction.
    pub fn has_resources(&self) -> bool {
        self.0
            .state
            .lock()
            .map(|s| !s.jobs.is_empty() || !s.fonts.is_empty())
            .unwrap_or(true)
    }
    pub fn measurement(&self, owner: ClientId) -> Arc<dyn TextMeasurement> {
        Arc::new(self.0.text.for_client(owner))
    }
    /// Poll completed workers without blocking the engine tick.
    pub fn advance(&self) -> Result<()> {
        let mut s = self
            .0
            .state
            .lock()
            .map_err(|_| anyhow::anyhow!("font queue poisoned"))?;
        let keys = s.jobs.keys().copied().collect::<Vec<_>>();
        for id in keys {
            let job = s.jobs.get_mut(&id).unwrap();
            let result: Result<Option<Arc<PreparedFontFace>>> = (|| match &mut job.stage {
                Stage::Read(task, permit) => {
                    if let Some(input) = task.try_take()? {
                        let index = job.index;
                        let task = permit.take().context("font CPU permit consumed")?.spawn(
                            move |cancel| {
                                cancel.check()?;
                                let mut face = PreparedFontFace::prepare(input.bytes, index)?;
                                face.retain_storage(input.charge);
                                cancel.check()?;
                                Ok(Arc::new(face))
                            },
                        )?;
                        job.stage = Stage::Prepare(task);
                    }
                    Ok(None)
                }
                Stage::Prepare(task) => task.try_take(),
                Stage::Done(_) => Ok(None),
            })();
            match result {
                Err(e) => {
                    s.jobs.get_mut(&id).unwrap().stage =
                        Stage::Done(FontLoadState::Failed(format!("{e:#}")));
                }
                Ok(Some(face)) => {
                    let (owner, name) = {
                        let j = &s.jobs[&id];
                        (j.owner, j.name.clone())
                    };
                    let mut identity = blake3::Hasher::new();
                    identity.update(name.as_bytes());
                    identity.update(&[0]);
                    identity.update(face.digest.as_bytes());
                    // Also fits UI/menu font aliases, whose shared bound is 128.
                    let canonical = format!("__font/{}/{}", owner.0, identity.finalize().to_hex());
                    let existing = s
                        .fonts
                        .values()
                        .find(|f| f.handle.owner == owner && f.alias == canonical)
                        .cloned();
                    let info = if let Some(info) = existing {
                        Ok(info)
                    } else {
                        (|| {
                            ensure!(
                                s.fonts.len() < 128
                                    && s.fonts.values().filter(|f| f.handle.owner == owner).count()
                                        < 64,
                                "font resource capacity"
                            );
                            self.0.text.publish_font(owner, &canonical, face.clone())?;
                            let info = FontInfo {
                                handle: FontHandle { owner, id },
                                logical_alias: name,
                                alias: canonical,
                                digest: face.digest.clone(),
                                revision: id,
                                families: face.families.clone(),
                                collection_faces: face.collection_faces,
                                face_index: face.face_index,
                                byte_len: face.byte_len(),
                            };
                            s.fonts.insert(id, info.clone());
                            Ok(info)
                        })()
                    };
                    s.jobs.get_mut(&id).unwrap().stage = Stage::Done(match info {
                        Ok(info) => FontLoadState::Ready(info),
                        Err(e) => FontLoadState::Failed(format!("{e:#}")),
                    });
                }
                Ok(None) => {}
            }
        }
        Ok(())
    }
}
struct ClientFonts {
    resources: FontResources,
    owner: ClientId,
    storage: Arc<dyn ClientStorage>,
}
impl FontLibrary for ClientFonts {
    fn begin(&self, spec: FontLoadSpec) -> crysirror_core::Result<FontJob> {
        if spec.name.is_empty()
            || spec.name.len() > 256
            || spec.name.starts_with("__font/")
            || spec.name.chars().any(char::is_control)
        {
            return Err(crysirror_core::Error::Invalid("font logical alias"));
        }
        let mut s = self.resources.0.state.lock().map_err(err)?;
        if s.jobs.len() >= 64 || s.jobs.values().filter(|j| j.owner == self.owner).count() >= 16 {
            return Err(crysirror_core::Error::Limit("font job capacity"));
        }
        let permit = self.resources.0.cpu.reserve().map_err(err)?;
        let used = self.resources.0.used.clone();
        let storage = self.storage.clone();
        // Charge supplied bytes before queueing: queued jobs also retain memory.
        let supplied_charge = match &spec.source {
            FontLoadSource::Bytes(bytes) => Some(charge(&used, bytes.len()).map_err(err)?),
            FontLoadSource::Storage(_) => None,
        };
        let source = spec.source;
        let read = self
            .resources
            .0
            .io
            .spawn(move |cancel| {
                cancel.check()?;
                match source {
                    FontLoadSource::Bytes(bytes) => {
                        let charge = supplied_charge.context("font bytes reservation")?;
                        Ok(Loaded { bytes, charge })
                    }
                    FontLoadSource::Storage(path) => {
                        let reader = storage.open_read(&path)?;
                        let len = usize::try_from(reader.len()?)?;
                        let charge = charge(&used, len)?;
                        let mut bytes = vec![0; len];
                        let mut offset = 0;
                        while offset < len {
                            cancel.check()?;
                            let n = reader.read_at(offset as u64, &mut bytes[offset..])?;
                            ensure!(n > 0 && n <= len - offset, "short/invalid font stream");
                            offset += n;
                        }
                        Ok(Loaded {
                            bytes: bytes.into(),
                            charge,
                        })
                    }
                }
            })
            .map_err(err)?;
        let id = s.next;
        s.next = s
            .next
            .checked_add(1)
            .filter(|v| *v < (1u64 << 53))
            .ok_or(crysirror_core::Error::Limit("font handle sequence"))?;
        s.jobs.insert(
            id,
            Job {
                owner: self.owner,
                name: spec.name,
                index: spec.face_index,
                stage: Stage::Read(read, Some(permit)),
            },
        );
        Ok(FontJob {
            owner: self.owner,
            id,
        })
    }
    fn status(&self, job: FontJob) -> crysirror_core::Result<FontLoadState> {
        if job.owner != self.owner {
            return Err(crysirror_core::Error::Invalid("foreign font job"));
        }
        self.resources.advance().map_err(err)?;
        let s = self.resources.0.state.lock().map_err(err)?;
        let j = s
            .jobs
            .get(&job.id)
            .filter(|j| j.owner == self.owner)
            .ok_or(crysirror_core::Error::NotFound)?;
        Ok(match &j.stage {
            Stage::Done(state) => state.clone(),
            _ => FontLoadState::Pending,
        })
    }
    fn resolve(&self, name: &str) -> crysirror_core::Result<Option<FontInfo>> {
        self.resources.advance().map_err(err)?;
        let s = self.resources.0.state.lock().map_err(err)?;
        Ok(s.fonts
            .values()
            .rev()
            .find(|f| {
                f.handle.owner == self.owner
                    && (f.alias == name
                        || f.logical_alias == name
                        || f.families.iter().any(|v| v.eq_ignore_ascii_case(name)))
            })
            .cloned())
    }
    fn is_loaded(&self, alias: &str) -> crysirror_core::Result<bool> {
        Ok(self
            .resources
            .0
            .text
            .for_client(self.owner)
            .font_digest(alias)
            .is_ok())
    }
    fn release_job(&self, job: FontJob) -> crysirror_core::Result<()> {
        let mut s = self.resources.0.state.lock().map_err(err)?;
        if job.owner != self.owner || !s.jobs.get(&job.id).is_some_and(|j| j.owner == self.owner) {
            return Err(crysirror_core::Error::NotFound);
        }
        s.jobs.remove(&job.id);
        Ok(())
    }
    fn release_font(&self, handle: FontHandle) -> crysirror_core::Result<()> {
        let mut s = self.resources.0.state.lock().map_err(err)?;
        let info = s
            .fonts
            .get(&handle.id)
            .filter(|f| handle.owner == self.owner && f.handle.owner == self.owner)
            .ok_or(crysirror_core::Error::NotFound)?;
        self.resources
            .0
            .text
            .unpublish_font(self.owner, &info.alias)
            .map_err(err)?;
        s.fonts.remove(&handle.id);
        for job in s.jobs.values_mut() {
            if matches!(&job.stage,Stage::Done(FontLoadState::Ready(info)) if info.handle==handle) {
                job.stage = Stage::Done(FontLoadState::Failed("font resource released".into()));
            }
        }
        Ok(())
    }
}

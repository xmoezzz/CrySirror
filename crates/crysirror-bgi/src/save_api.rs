//! Native save-file IO fences over the client's shared storage capability.
//! Encoding and installation remain the original VM's native save policy.
use super::*;
use crysirror_core::service::ClientStorage;
pub(crate) enum SaveWork {
    Read(Task<Option<Vec<u8>>>),
    Write(Task<bool>),
    Enumerate(Task<Vec<String>>),
}
fn read_loose(storage: &dyn ClientStorage, path: &VfsPath) -> Result<Option<Vec<u8>>> {
    match storage.metadata(path.mount(), path.relative()) {
        Ok(Some(metadata)) => {
            if metadata.is_directory {
                return Ok(None);
            }
            ensure!(
                metadata.size.is_none_or(|n| n <= 64 * 1024 * 1024),
                "BGI native save read limit"
            );
        }
        Ok(None) | Err(Error::NotFound) => return Ok(None),
        Err(Error::Unsupported(_)) => (),
        Err(error) => return Err(error.into()),
    }
    match storage.read(path) {
        Ok(bytes) => {
            ensure!(
                bytes.len() <= 64 * 1024 * 1024,
                "BGI native save read limit"
            );
            Ok(Some(bytes))
        }
        Err(Error::NotFound) => Ok(None),
        Err(error) => Err(error.into()),
    }
}
impl BgiVm {
    pub(crate) fn native_enumerate(
        &mut self,
        host: &mut dyn HostServices,
        vm: &mut Machine,
        request: u64,
        pattern: &str,
        recursive: bool,
        directories: bool,
    ) -> Result<bool> {
        let work = if let Some(work) = self.save_jobs.remove(&request) {
            work
        } else {
            let storage = host.storage(self.client)?;
            let config = self.config.clone();
            let pattern = pattern.to_owned();
            SaveWork::Enumerate(self.pool.spawn(move |cancel| {
                cancel.check()?;
                let entries = super::system_files::enumerate(
                    storage.as_ref(),
                    &config,
                    &pattern,
                    recursive,
                    directories,
                )?;
                cancel.check()?;
                Ok(entries)
            })?)
        };
        let SaveWork::Enumerate(mut task) = work else {
            bail!("BGI enumeration continuation kind mismatch")
        };
        let Some(entries) = task.try_take()? else {
            self.save_jobs.insert(request, SaveWork::Enumerate(task));
            return Ok(false);
        };
        vm.commit_enumeration(entries)?;
        Ok(true)
    }
    pub(crate) fn native_file_read(
        &mut self,
        host: &mut dyn HostServices,
        vm: &mut Machine,
        request: u64,
        path: &str,
    ) -> Result<bool> {
        let work = if let Some(work) = self.save_jobs.remove(&request) {
            work
        } else {
            let storage = host.storage(self.client)?;
            let relative = program_io::relative(&self.config, path)?;
            let asset = self.config.asset(&relative)?;
            SaveWork::Read(self.pool.spawn(move |cancel| {
                cancel.check()?;
                let save = VfsPath::new(Mount::Save, &relative)?;
                let bytes = match read_loose(storage.as_ref(), &save)? {
                    Some(bytes) => Some(bytes),
                    None => read_loose(storage.as_ref(), &VfsPath::new(Mount::Asset, &asset)?)?,
                };
                cancel.check()?;
                Ok(bytes)
            })?)
        };
        let SaveWork::Read(mut task) = work else {
            bail!("BGI native read continuation kind mismatch")
        };
        let Some(bytes) = task.try_take()? else {
            self.save_jobs.insert(request, SaveWork::Read(task));
            return Ok(false);
        };
        vm.commit_file_read(bytes)?;
        Ok(true)
    }
    pub(crate) fn native_file_write(
        &mut self,
        host: &mut dyn HostServices,
        vm: &mut Machine,
        request: u64,
        path: &str,
        bytes: Arc<[u8]>,
    ) -> Result<bool> {
        let work = if let Some(work) = self.save_jobs.remove(&request) {
            work
        } else {
            ensure!(
                bytes.len() <= 64 * 1024 * 1024,
                "BGI native save write limit"
            );
            let storage = host.storage(self.client)?;
            let path = VfsPath::new(Mount::Save, &program_io::relative(&self.config, path)?)?;
            SaveWork::Write(self.pool.spawn(move |cancel| {
                cancel.check()?;
                let committed = match storage.write_atomic_existing_parent(&path, &bytes) {
                    Ok(()) => true,
                    Err(error @ Error::Unsupported(_)) => return Err(error.into()),
                    Err(_) => false,
                };
                cancel.check()?;
                Ok(committed)
            })?)
        };
        let SaveWork::Write(mut task) = work else {
            bail!("BGI native write continuation kind mismatch")
        };
        let Some(committed) = task.try_take()? else {
            self.save_jobs.insert(request, SaveWork::Write(task));
            return Ok(false);
        };
        vm.commit_file_write(committed)?;
        Ok(true)
    }
}

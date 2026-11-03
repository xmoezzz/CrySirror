//! Privileged presenting-host path context. Never exposed by ClientStorage.
use super::*;
#[derive(Clone, Debug)]
pub struct DesktopStorageRoots {
    pub assets: PathBuf,
    pub save: PathBuf,
    pub cache: PathBuf,
}
impl DesktopStorageRoots {
    pub fn get(&self, mount: Mount) -> &Path {
        match mount {
            Mount::Asset => &self.assets,
            Mount::Save => &self.save,
            Mount::Cache => &self.cache,
        }
    }
}

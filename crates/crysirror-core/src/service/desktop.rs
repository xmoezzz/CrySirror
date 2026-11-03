//! Asynchronous desktop capabilities. A missing provider is Unsupported;
//! cancellation is a distinct successful chooser response.
use crate::{
    Error, Result,
    path::{Mount, VfsPath},
    state::ClientId,
};
use alloc::{boxed::Box, string::String, vec::Vec};
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DesktopPath {
    pub mount: Mount,
    pub relative: String,
}
impl DesktopPath {
    pub fn validate(&self) -> Result<()> {
        if !self.relative.is_empty()
            && VfsPath::new(self.mount, &self.relative)?.relative() != self.relative
        {
            return Err(Error::Invalid("desktop capability path"));
        }
        Ok(())
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DesktopDialogKind {
    OpenFile,
    SaveFile,
    Folder,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DesktopSpecialFolder {
    Windows,
    Desktop,
    Programs,
    Documents,
    ProgramFiles,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DesktopDialogFilter {
    pub label: String,
    pub extension: String,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DesktopRequest {
    ClipboardWrite {
        text: String,
    },
    UserName,
    ComputerName,
    SpecialFolder {
        folder: DesktopSpecialFolder,
    },
    FileDialog {
        kind: DesktopDialogKind,
        title: String,
        initial: Option<DesktopPath>,
        initial_special: Option<DesktopSpecialFolder>,
        default_name: String,
        filters: Vec<DesktopDialogFilter>,
    },
}
impl DesktopRequest {
    pub fn validate(&self) -> Result<()> {
        let text = |s: &str, max| -> Result<()> {
            if s.len() > max || s.contains('\0') {
                Err(Error::Invalid("desktop request text"))
            } else {
                Ok(())
            }
        };
        match self {
            Self::ClipboardWrite { text: s } => text(s, 1024 * 1024)?,
            Self::FileDialog {
                title,
                initial,
                default_name,
                filters,
                ..
            } => {
                text(title, 8192)?;
                text(default_name, 4096)?;
                if default_name.contains(['/', '\\']) {
                    return Err(Error::Invalid("desktop default filename"));
                }
                if let Some(path) = initial {
                    path.validate()?;
                }
                if filters.len() > 256 {
                    return Err(Error::Limit("desktop file filters"));
                }
                for filter in filters {
                    text(&filter.label, 1024)?;
                    text(&filter.extension, 256)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DesktopResponse {
    ClipboardWritten,
    Text(String),
    Selection(Option<DesktopPath>),
    SpecialFolder(Option<DesktopPath>),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DesktopRequestState {
    Pending,
    Ready(DesktopResponse),
}
pub trait DesktopRequestJob: Send {
    fn poll(&mut self) -> Result<DesktopRequestState>;
}
pub trait DesktopProvider: Send + Sync {
    fn begin(
        &self,
        client: ClientId,
        request: DesktopRequest,
    ) -> Result<Box<dyn DesktopRequestJob>>;
}

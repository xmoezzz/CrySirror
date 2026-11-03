//! Native platform value/status policy over shared asynchronous host services.
use super::*;
use crysirror_bgi_vm::{NativePlatformRequest as N, NativePlatformResponse as R};
use crysirror_core::service::{
    ClientStorage, DesktopDialogFilter, DesktopDialogKind as Kind, DesktopPath,
    DesktopRequest as Request, DesktopRequestJob, DesktopRequestState, DesktopResponse as Response,
    DesktopSpecialFolder as Folder,
};
pub(crate) enum PlatformWork {
    Desktop(Box<dyn DesktopRequestJob>),
    Mapping(Task<R>),
}
fn special(mode: i32) -> Option<Folder> {
    match mode {
        0 => Some(Folder::Windows),
        1 => Some(Folder::Desktop),
        2 => Some(Folder::Programs),
        3 => Some(Folder::Documents),
        4 | 5 => Some(Folder::ProgramFiles),
        _ => None,
    }
}
fn initial(config: &Config, path: &str, save: bool) -> Result<DesktopPath> {
    let relative = super::system_files::relative(config, path)?;
    let mount = if save { Mount::Save } else { Mount::Asset };
    let relative = if mount == Mount::Asset {
        if relative.is_empty() {
            config.root.clone()
        } else {
            config.asset(&relative)?
        }
    } else {
        relative
    };
    let result = DesktopPath { mount, relative };
    result.validate()?;
    Ok(result)
}
fn path_name(
    config: &Config,
    path: DesktopPath,
    storage: &dyn ClientStorage,
    limit: usize,
) -> Result<String> {
    path.validate()?;
    ensure!(
        matches!(path.mount, Mount::Asset | Mount::Save),
        "BGI chooser cannot address a cache mount"
    );
    let relative = if path.mount == Mount::Asset && !config.root.is_empty() {
        if path.relative == config.root {
            String::new()
        } else {
            path.relative
                .strip_prefix(&format!("{}/", config.root))
                .context("selected path is outside the configured BGI game root")?
                .to_owned()
        }
    } else {
        path.relative
    };
    if path.mount == Mount::Asset
        && !relative.is_empty()
        && storage.metadata(Mount::Save, &relative)?.is_some()
    {
        bail!("selected asset is shadowed by a distinct native save path");
    }
    let name = if relative.is_empty() {
        "C:\\CrySirror\\".to_owned()
    } else {
        format!("C:\\CrySirror\\{}", relative.replace('/', "\\"))
    };
    let (encoded, _, errors) = encoding_rs::SHIFT_JIS.encode(&name);
    ensure!(
        !errors,
        "selected path is not representable in native CP932"
    );
    ensure!(
        encoded.len() < limit,
        "selected path exceeds the native chooser buffer"
    );
    Ok(name)
}
fn translate(
    request: &N,
    response: Response,
    config: &Config,
    storage: &dyn ClientStorage,
) -> Result<R> {
    Ok(match (request, response) {
        (N::ClipboardSet { .. }, Response::ClipboardWritten) => R::ClipboardSet(true),
        (N::UserName | N::ComputerName, Response::Text(text)) => R::Text(text),
        (N::SpecialFolder { .. }, Response::SpecialFolder(path)) => R::SpecialFolder(
            path.map(|p| path_name(config, p, storage, 260))
                .transpose()?,
        ),
        (N::FileDialog { .. } | N::ResourceFileDialog { .. }, Response::Selection(path)) => {
            R::FileDialog(Ok(path
                .map(|p| path_name(config, p, storage, 780))
                .transpose()?))
        }
        (N::FolderDialog { .. }, Response::Selection(path)) => R::FolderDialog(
            path.map(|p| path_name(config, p, storage, 260))
                .transpose()?,
        ),
        _ => bail!("desktop host response does not match the native request"),
    })
}
impl BgiVm {
    pub(crate) fn native_platform_call(
        &mut self,
        host: &mut dyn HostServices,
        vm: &mut Machine,
        request_id: u64,
        request: &N,
    ) -> Result<bool> {
        let work = if let Some(work) = self.desktop_jobs.remove(&request_id) {
            work
        } else {
            let desktop = match request {
                N::ClipboardSet { text } => Request::ClipboardWrite { text: text.clone() },
                N::UserName => Request::UserName,
                N::ComputerName => Request::ComputerName,
                N::SpecialFolder { mode } => {
                    let Some(folder) = special(*mode) else {
                        vm.commit_native_platform(R::SpecialFolder(None))?;
                        return Ok(true);
                    };
                    Request::SpecialFolder { folder }
                }
                N::FileDialog {
                    initial_directory,
                    description,
                    extension,
                    title,
                    mode,
                } => {
                    if !matches!(mode, 0 | 1) {
                        vm.commit_native_platform(R::FileDialog(Err(4)))?;
                        return Ok(true);
                    }
                    Request::FileDialog {
                        kind: if *mode == 0 {
                            Kind::OpenFile
                        } else {
                            Kind::SaveFile
                        },
                        title: title.clone(),
                        initial: Some(initial(&self.config, initial_directory, *mode == 1)?),
                        initial_special: None,
                        default_name: String::new(),
                        filters: vec![DesktopDialogFilter {
                            label: description.clone(),
                            extension: extension.clone(),
                        }],
                    }
                }
                N::ResourceFileDialog {
                    mode,
                    title,
                    default_name,
                    filters,
                } => {
                    if !matches!(mode, 0 | 1) {
                        vm.commit_native_platform(R::FileDialog(Err(4)))?;
                        return Ok(true);
                    }
                    if filters.is_empty() {
                        vm.commit_native_platform(R::FileDialog(Err(7)))?;
                        return Ok(true);
                    }
                    let normalized = default_name.replace('\\', "/");
                    let (folder, file) = normalized.rsplit_once('/').unwrap_or(("", &normalized));
                    Request::FileDialog {
                        kind: if *mode == 0 {
                            Kind::OpenFile
                        } else {
                            Kind::SaveFile
                        },
                        title: title.clone(),
                        initial: Some(initial(&self.config, folder, *mode == 1)?),
                        initial_special: None,
                        default_name: file.into(),
                        filters: filters
                            .iter()
                            .map(|(label, extension)| DesktopDialogFilter {
                                label: label.clone(),
                                extension: extension.clone(),
                            })
                            .collect(),
                    }
                }
                N::FolderDialog { title, root } => Request::FileDialog {
                    kind: Kind::Folder,
                    title: title.clone(),
                    initial: None,
                    initial_special: special(*root),
                    default_name: String::new(),
                    filters: vec![],
                },
            };
            PlatformWork::Desktop(host.begin_desktop_request(self.client, desktop)?)
        };
        let work = match work {
            PlatformWork::Desktop(mut job) => {
                let response = match job.poll() {
                    Ok(DesktopRequestState::Pending) => {
                        self.desktop_jobs
                            .insert(request_id, PlatformWork::Desktop(job));
                        return Ok(false);
                    }
                    Ok(DesktopRequestState::Ready(response)) => response,
                    Err(error)
                        if matches!(request, N::ClipboardSet { .. })
                            && !matches!(error, Error::Unsupported(_)) =>
                    {
                        vm.commit_native_platform(R::ClipboardSet(false))?;
                        return Ok(true);
                    }
                    Err(error) => return Err(error.into()),
                };
                let request = request.clone();
                let config = self.config.clone();
                let storage = host.storage(self.client)?;
                PlatformWork::Mapping(self.pool.spawn(move |cancel| {
                    cancel.check()?;
                    let result = translate(&request, response, &config, storage.as_ref())?;
                    cancel.check()?;
                    Ok(result)
                })?)
            }
            other => other,
        };
        let PlatformWork::Mapping(mut task) = work else {
            unreachable!()
        };
        let Some(response) = task.try_take()? else {
            self.desktop_jobs
                .insert(request_id, PlatformWork::Mapping(task));
            return Ok(false);
        };
        vm.commit_native_platform(response)?;
        Ok(true)
    }
}

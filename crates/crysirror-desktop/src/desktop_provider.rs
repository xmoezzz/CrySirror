//! Presenting-host desktop operations. Worker ownership keeps OS dialogs and
//! clipboard I/O out of every VM and out of the event-loop tick.
use crysirror_core::{Error, Result, path::Mount, service::*, state::ClientId};
use crysirror_runtime::vfs::DesktopStorageRoots;
use crysirror_tasks::{Task, TaskPool};
use std::{
    collections::BTreeMap,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    sync::{Arc, Mutex},
    time::Duration,
};
pub(crate) struct Provider {
    roots: Arc<BTreeMap<ClientId, DesktopStorageRoots>>,
    tasks: TaskPool,
    dialogs: Arc<Mutex<()>>,
    allow_ui: bool,
}
impl Provider {
    pub(crate) fn new(
        roots: BTreeMap<ClientId, DesktopStorageRoots>,
        allow_ui: bool,
    ) -> anyhow::Result<Self> {
        Ok(Self {
            roots: Arc::new(roots),
            tasks: TaskPool::new("desktop-platform", 2, 32)?,
            dialogs: Arc::new(Mutex::new(())),
            allow_ui,
        })
    }
}
struct Job {
    task: Option<Task<Result<DesktopResponse>>>,
    result: Option<Result<DesktopResponse>>,
}
impl DesktopRequestJob for Job {
    fn poll(&mut self) -> Result<DesktopRequestState> {
        if self.result.is_none() {
            match self
                .task
                .as_mut()
                .expect("desktop request state")
                .try_take()
            {
                Ok(None) => return Ok(DesktopRequestState::Pending),
                Ok(Some(result)) => self.result = Some(result),
                Err(error) => self.result = Some(Err(Error::Io(error.to_string()))),
            }
            self.task = None;
        }
        self.result
            .as_ref()
            .unwrap()
            .clone()
            .map(DesktopRequestState::Ready)
    }
}
impl DesktopProvider for Provider {
    fn begin(
        &self,
        client: ClientId,
        request: DesktopRequest,
    ) -> Result<Box<dyn DesktopRequestJob>> {
        request.validate()?;
        if !self.allow_ui
            && matches!(
                &request,
                DesktopRequest::FileDialog { .. } | DesktopRequest::ClipboardWrite { .. }
            )
        {
            return Err(Error::Unsupported(
                "interactive desktop capability requires a presenter",
            ));
        }
        let roots = self.roots.get(&client).cloned().ok_or(Error::NotFound)?;
        let dialogs = self.dialogs.clone();
        let task = self
            .tasks
            .spawn(move |cancel| {
                let check = || cancel.check().map_err(|e| Error::Io(e.to_string()));
                check()?;
                let result = match &request {
                    DesktopRequest::FileDialog { .. } => {
                        // Only one OS-owned modal chooser at once. Polling the
                        // mutex, rather than lock(), keeps cancellation responsive.
                        let guard = loop {
                            match dialogs.try_lock() {
                                Ok(guard) => break guard,
                                Err(std::sync::TryLockError::WouldBlock) => {
                                    check()?;
                                    std::thread::sleep(Duration::from_millis(10));
                                }
                                Err(_) => {
                                    return Ok(Err(Error::Io(
                                        "desktop dialog ownership poisoned".into(),
                                    )));
                                }
                            }
                        };
                        let result = perform(request, &roots, &check);
                        drop(guard);
                        result
                    }
                    _ => perform(request, &roots, &check),
                };
                Ok(result)
            })
            .map_err(|e| Error::Io(e.to_string()))?;
        Ok(Box::new(Job {
            task: Some(task),
            result: None,
        }))
    }
}
fn io(error: std::io::Error) -> Error {
    Error::Io(error.to_string())
}
/// All programs and script bodies are host constants. Game strings are passed
/// as distinct argv/environment/stdin values, never interpolated into code.
fn run(
    mut command: Command,
    input: Option<&str>,
    check: &dyn Fn() -> Result<()>,
) -> Result<Output> {
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    if input.is_some() {
        command.stdin(Stdio::piped());
    } else {
        command.stdin(Stdio::null());
    }
    let mut child = command.spawn().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            Error::Unsupported("desktop executable is unavailable")
        } else {
            io(e)
        }
    })?;
    let writer = if let Some(text) = input {
        let mut stream = child
            .stdin
            .take()
            .ok_or(Error::Io("desktop child input".into()))?;
        let text = text.as_bytes().to_vec();
        Some(std::thread::spawn(move || stream.write_all(&text)))
    } else {
        None
    };
    loop {
        if let Err(error) = check() {
            let _ = child.kill();
            let _ = child.wait();
            if let Some(writer) = writer {
                let _ = writer.join();
            }
            return Err(error);
        }
        if child.try_wait().map_err(io)?.is_some() {
            let output = child.wait_with_output().map_err(io)?;
            if let Some(writer) = writer {
                writer
                    .join()
                    .map_err(|_| Error::Io("desktop input worker panicked".into()))?
                    .map_err(io)?;
            }
            return Ok(output);
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}
fn failure(output: &Output) -> Error {
    Error::Io(format!(
        "desktop operation exited with {}: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
            .chars()
            .take(1024)
            .collect::<String>()
    ))
}
fn text_output(output: &Output) -> Result<String> {
    String::from_utf8(output.stdout.clone())
        .map(|s| s.trim_end_matches(['\r', '\n']).to_owned())
        .map_err(|_| Error::Invalid("desktop response text encoding"))
}
fn special(folder: DesktopSpecialFolder) -> Option<PathBuf> {
    let home = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from);
    match folder {
        DesktopSpecialFolder::Windows => std::env::var_os("WINDIR").map(PathBuf::from),
        DesktopSpecialFolder::Desktop => home.map(|p| p.join("Desktop")),
        DesktopSpecialFolder::Documents => home.map(|p| p.join("Documents")),
        DesktopSpecialFolder::ProgramFiles => std::env::var_os("ProgramFiles").map(PathBuf::from),
        DesktopSpecialFolder::Programs => {
            #[cfg(target_os = "windows")]
            {
                std::env::var_os("APPDATA")
                    .map(|p| PathBuf::from(p).join("Microsoft/Windows/Start Menu/Programs"))
            }
            #[cfg(not(target_os = "windows"))]
            {
                home.map(|p| p.join(".local/share/applications"))
            }
        }
    }
}
fn map_selection(roots: &DesktopStorageRoots, path: &Path, save: bool) -> Result<DesktopPath> {
    let selected = if path.exists() {
        path.canonicalize().map_err(io)?
    } else if save {
        let leaf = path
            .file_name()
            .ok_or(Error::Invalid("desktop save filename"))?;
        path.parent()
            .ok_or(Error::Invalid("desktop save parent"))?
            .canonicalize()
            .map_err(io)?
            .join(leaf)
    } else {
        return Err(Error::NotFound);
    };
    let mounts = if save {
        &[Mount::Save][..]
    } else {
        &[Mount::Save, Mount::Asset][..]
    };
    for &mount in mounts {
        if let Ok(relative) = selected.strip_prefix(roots.get(mount)) {
            let relative = relative
                .to_str()
                .ok_or(Error::Unsupported("non-Unicode selected path"))?
                .replace(std::path::MAIN_SEPARATOR, "/");
            let result = DesktopPath { mount, relative };
            result.validate()?;
            return Ok(result);
        }
    }
    Err(Error::Unsupported(
        "selected path is outside this client's storage capabilities",
    ))
}
fn perform(
    request: DesktopRequest,
    roots: &DesktopStorageRoots,
    check: &dyn Fn() -> Result<()>,
) -> Result<DesktopResponse> {
    match request {
        DesktopRequest::ClipboardWrite { text } => {
            clipboard(&text, check)?;
            Ok(DesktopResponse::ClipboardWritten)
        }
        DesktopRequest::UserName => Ok(DesktopResponse::Text(
            std::env::var("USERNAME")
                .or_else(|_| std::env::var("USER"))
                .map_err(|_| Error::Unsupported("host user-name query"))?,
        )),
        DesktopRequest::ComputerName => {
            let name = if let Ok(name) = std::env::var("COMPUTERNAME") {
                name
            } else {
                #[cfg(target_os = "windows")]
                let mut command = Command::new(
                    PathBuf::from(
                        std::env::var_os("SystemRoot")
                            .ok_or(Error::Unsupported("Windows system command root"))?,
                    )
                    .join("System32/hostname.exe"),
                );
                #[cfg(not(target_os = "windows"))]
                let mut command = Command::new("/bin/hostname");
                #[cfg(not(target_os = "windows"))]
                command.arg("-s");
                let output = run(command, None, check)?;
                if !output.status.success() {
                    return Err(failure(&output));
                }
                text_output(&output)?
            };
            Ok(DesktopResponse::Text(
                name.split('.')
                    .next()
                    .unwrap_or("")
                    .to_ascii_uppercase()
                    .chars()
                    .take(15)
                    .collect(),
            ))
        }
        DesktopRequest::SpecialFolder { folder } => {
            let path = special(folder)
                .map(|p| map_selection(roots, &p, false))
                .transpose()?;
            Ok(DesktopResponse::SpecialFolder(path))
        }
        DesktopRequest::FileDialog {
            kind,
            title,
            initial,
            initial_special,
            default_name,
            filters,
        } => {
            let initial = if let Some(path) = initial {
                roots.get(path.mount).join(&path.relative)
            } else if let Some(folder) = initial_special {
                special(folder).unwrap_or_else(|| roots.assets.clone())
            } else if kind == DesktopDialogKind::SaveFile {
                roots.save.clone()
            } else {
                roots.assets.clone()
            };
            let selected = choose(kind, &title, &initial, &default_name, &filters, check)?;
            let selected = selected
                .map(|p| {
                    let path = map_selection(roots, &p, kind == DesktopDialogKind::SaveFile)?;
                    let physical = roots.get(path.mount).join(&path.relative);
                    match kind {
                        DesktopDialogKind::Folder => {
                            if !physical.is_dir() {
                                return Err(Error::Invalid("chooser selected a non-directory"));
                            }
                        }
                        DesktopDialogKind::OpenFile => {
                            if !physical.is_file() {
                                return Err(Error::Invalid("chooser selected a non-file"));
                            }
                        }
                        DesktopDialogKind::SaveFile => {
                            if physical.exists() && !physical.is_file() {
                                return Err(Error::Invalid("save chooser selected a non-file"));
                            }
                        }
                    }
                    Ok(path)
                })
                .transpose()?;
            Ok(DesktopResponse::Selection(selected))
        }
    }
}
#[cfg(target_os = "macos")]
fn clipboard(text: &str, check: &dyn Fn() -> Result<()>) -> Result<()> {
    let output = run(Command::new("/usr/bin/pbcopy"), Some(text), check)?;
    if output.status.success() {
        Ok(())
    } else {
        Err(failure(&output))
    }
}
#[cfg(target_os = "windows")]
fn powershell() -> Result<Command> {
    let root = std::env::var_os("SystemRoot")
        .or_else(|| std::env::var_os("WINDIR"))
        .ok_or(Error::Unsupported("Windows system command root"))?;
    Ok(Command::new(
        PathBuf::from(root).join("System32/WindowsPowerShell/v1.0/powershell.exe"),
    ))
}
#[cfg(target_os = "windows")]
fn clipboard(text: &str, check: &dyn Fn() -> Result<()>) -> Result<()> {
    let mut command = powershell()?;
    command.args(["-NoProfile","-NonInteractive","-Command",r#"[Console]::InputEncoding = [System.Text.Encoding]::UTF8; Set-Clipboard -Value ([Console]::In.ReadToEnd()); if (!$?) { exit 3 }"#]);
    let output = run(command, Some(text), check)?;
    if output.status.success() {
        Ok(())
    } else {
        Err(failure(&output))
    }
}
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn clipboard(text: &str, check: &dyn Fn() -> Result<()>) -> Result<()> {
    for (program, args) in [
        ("wl-copy", Vec::<&str>::new()),
        ("xclip", vec!["-selection", "clipboard"]),
    ] {
        let mut command = Command::new(program);
        command.args(args);
        match run(command, Some(text), check) {
            Ok(output) if output.status.success() => return Ok(()),
            Err(Error::Unsupported(_)) => continue,
            Ok(output) => return Err(failure(&output)),
            Err(error) => return Err(error),
        }
    }
    Err(Error::Unsupported("system clipboard backend"))
}
#[cfg(target_os = "macos")]
fn choose(
    kind: DesktopDialogKind,
    title: &str,
    initial: &Path,
    default_name: &str,
    _filters: &[DesktopDialogFilter],
    check: &dyn Fn() -> Result<()>,
) -> Result<Option<PathBuf>> {
    let script = match kind {
        DesktopDialogKind::OpenFile => {
            r#"on run argv
set selected to choose file with prompt (item 1 of argv) default location POSIX file (item 2 of argv)
return POSIX path of selected
end run"#
        }
        DesktopDialogKind::SaveFile => {
            r#"on run argv
set selected to choose file name with prompt (item 1 of argv) default location POSIX file (item 2 of argv) default name (item 3 of argv)
return POSIX path of selected
end run"#
        }
        DesktopDialogKind::Folder => {
            r#"on run argv
set selected to choose folder with prompt (item 1 of argv) default location POSIX file (item 2 of argv)
return POSIX path of selected
end run"#
        }
    };
    let mut command = Command::new("/usr/bin/osascript");
    command
        .args(["-e", script, "--", title])
        .arg(initial)
        .arg(default_name);
    let output = run(command, None, check)?;
    if output.status.success() {
        let value = text_output(&output)?;
        if value.is_empty() {
            return Err(Error::Invalid("empty native chooser selection"));
        }
        return Ok(Some(value.into()));
    }
    if output.status.code() == Some(1) && String::from_utf8_lossy(&output.stderr).contains("(-128)")
    {
        return Ok(None);
    }
    Err(failure(&output))
}
#[cfg(target_os = "windows")]
fn choose(
    kind: DesktopDialogKind,
    title: &str,
    initial: &Path,
    default_name: &str,
    filters: &[DesktopDialogFilter],
    check: &dyn Fn() -> Result<()>,
) -> Result<Option<PathBuf>> {
    let script = match kind {
        DesktopDialogKind::OpenFile => {
            r#"$ErrorActionPreference='Stop'; Add-Type -AssemblyName System.Windows.Forms; $d=New-Object System.Windows.Forms.OpenFileDialog; $d.Title=$env:CRYSIRROR_DIALOG_TITLE; $d.InitialDirectory=$env:CRYSIRROR_DIALOG_INITIAL; $d.FileName=$env:CRYSIRROR_DIALOG_DEFAULT; $d.Filter=$env:CRYSIRROR_DIALOG_FILTER; if($d.ShowDialog() -eq [System.Windows.Forms.DialogResult]::OK){[Console]::OutputEncoding=[System.Text.Encoding]::UTF8;[Console]::Write($d.FileName);exit 0};exit 2"#
        }
        DesktopDialogKind::SaveFile => {
            r#"$ErrorActionPreference='Stop'; Add-Type -AssemblyName System.Windows.Forms; $d=New-Object System.Windows.Forms.SaveFileDialog; $d.Title=$env:CRYSIRROR_DIALOG_TITLE; $d.InitialDirectory=$env:CRYSIRROR_DIALOG_INITIAL; $d.FileName=$env:CRYSIRROR_DIALOG_DEFAULT; $d.Filter=$env:CRYSIRROR_DIALOG_FILTER; if($d.ShowDialog() -eq [System.Windows.Forms.DialogResult]::OK){[Console]::OutputEncoding=[System.Text.Encoding]::UTF8;[Console]::Write($d.FileName);exit 0};exit 2"#
        }
        DesktopDialogKind::Folder => {
            r#"$ErrorActionPreference='Stop'; Add-Type -AssemblyName System.Windows.Forms; $d=New-Object System.Windows.Forms.FolderBrowserDialog; $d.Description=$env:CRYSIRROR_DIALOG_TITLE; $d.SelectedPath=$env:CRYSIRROR_DIALOG_INITIAL; if($d.ShowDialog() -eq [System.Windows.Forms.DialogResult]::OK){[Console]::OutputEncoding=[System.Text.Encoding]::UTF8;[Console]::Write($d.SelectedPath);exit 0};exit 2"#
        }
    };
    let filter = filters
        .iter()
        .flat_map(|f| {
            let extension = f
                .extension
                .trim()
                .trim_start_matches("*.")
                .trim_start_matches('.');
            [
                f.label.clone(),
                if extension.is_empty() {
                    "*.*".into()
                } else {
                    format!("*.{extension}")
                },
            ]
        })
        .chain(["All files (*.*)".into(), "*.*".into()])
        .collect::<Vec<_>>()
        .join("|");
    let mut command = powershell()?;
    command
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .env("CRYSIRROR_DIALOG_TITLE", title)
        .env("CRYSIRROR_DIALOG_INITIAL", initial)
        .env("CRYSIRROR_DIALOG_DEFAULT", default_name)
        .env("CRYSIRROR_DIALOG_FILTER", filter);
    let output = run(command, None, check)?;
    if output.status.success() {
        let value = text_output(&output)?;
        if value.is_empty() {
            return Err(Error::Invalid("empty native chooser selection"));
        }
        Ok(Some(value.into()))
    } else if output.status.code() == Some(2) {
        Ok(None)
    } else {
        Err(failure(&output))
    }
}
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn choose(
    kind: DesktopDialogKind,
    title: &str,
    initial: &Path,
    default_name: &str,
    filters: &[DesktopDialogFilter],
    check: &dyn Fn() -> Result<()>,
) -> Result<Option<PathBuf>> {
    let selected = if default_name.is_empty() {
        initial.to_owned()
    } else {
        initial.join(default_name)
    };
    let mut zenity = Command::new("zenity");
    zenity
        .arg("--file-selection")
        .arg(format!("--title={title}"))
        .arg(format!("--filename={}", selected.display()));
    match kind {
        DesktopDialogKind::OpenFile => {}
        DesktopDialogKind::SaveFile => {
            zenity.args(["--save", "--confirm-overwrite"]);
        }
        DesktopDialogKind::Folder => {
            zenity.arg("--directory");
        }
    }
    for filter in filters {
        let ext = filter
            .extension
            .trim()
            .trim_start_matches("*.")
            .trim_start_matches('.');
        if !ext.is_empty() {
            zenity.arg(format!("--file-filter={} | *.{ext}", filter.label));
        }
    }
    let output = match run(zenity, None, check) {
        Err(Error::Unsupported(_)) => {
            let mut command = Command::new("kdialog");
            command
                .arg(match kind {
                    DesktopDialogKind::OpenFile => "--getopenfilename",
                    DesktopDialogKind::SaveFile => "--getsavefilename",
                    DesktopDialogKind::Folder => "--getexistingdirectory",
                })
                .arg(selected)
                .arg("--title")
                .arg(title);
            run(command, None, check)?
        }
        other => other?,
    };
    if output.status.success() {
        let value = text_output(&output)?;
        if value.is_empty() {
            return Err(Error::Invalid("empty native chooser selection"));
        }
        Ok(Some(value.into()))
    } else if output.status.code() == Some(1) && output.stderr.is_empty() {
        Ok(None)
    } else {
        Err(failure(&output))
    }
}

pub(crate) fn install(
    runtime: &mut crysirror_runtime::Runtime,
    allow_ui: bool,
) -> anyhow::Result<()> {
    let roots = (0..runtime.project.clients.len())
        .map(|index| {
            let client = ClientId((index + 1) as u32);
            runtime
                .host
                .fs
                .desktop_roots(client)
                .map(|roots| (client, roots))
        })
        .collect::<Result<BTreeMap<_, _>>>()?;
    runtime.host.desktop_provider = Some(Arc::new(Provider::new(roots, allow_ui)?));
    Ok(())
}

//! Race-free Windows Job Object assignment for a caller-owned spawn plan.
#![cfg(windows)]
#![expect(
    unsafe_code,
    reason = "ADR-0042: the native Windows lifecycle boundary"
)]

use std::{
    cmp::Ordering,
    ffi::{OsStr, OsString, c_void},
    fs::File,
    io,
    mem::{size_of, size_of_val},
    os::windows::{
        ffi::{OsStrExt, OsStringExt},
        io::{AsRawHandle, FromRawHandle, OwnedHandle},
    },
    path::{Path, PathBuf},
    ptr::{null, null_mut},
};
#[cfg(any(test, feature = "test-support"))]
use windows_sys::Win32::System::Console::{
    CONSOLE_SCREEN_BUFFER_INFO, GetConsoleScreenBufferInfo, GetStdHandle, STD_OUTPUT_HANDLE,
};
use windows_sys::Win32::{
    Foundation::{
        DUPLICATE_SAME_ACCESS, DuplicateHandle, HANDLE, INVALID_HANDLE_VALUE, WAIT_OBJECT_0,
        WAIT_TIMEOUT,
    },
    Globalization::{CSTR_EQUAL, CSTR_GREATER_THAN, CSTR_LESS_THAN, CompareStringOrdinal},
    System::{
        Console::{COORD, ClosePseudoConsole, CreatePseudoConsole, HPCON, ResizePseudoConsole},
        JobObjects::{
            CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            JOBOBJECT_BASIC_ACCOUNTING_INFORMATION, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
            JobObjectBasicAccountingInformation, JobObjectExtendedLimitInformation,
            QueryInformationJobObject, SetInformationJobObject, TerminateJobObject,
        },
        Pipes::CreatePipe,
        SystemInformation::{GetSystemDirectoryW, GetWindowsDirectoryW},
        Threading::{
            CREATE_NO_WINDOW, CREATE_UNICODE_ENVIRONMENT, CreateProcessW,
            DeleteProcThreadAttributeList, EXTENDED_STARTUPINFO_PRESENT, GetCurrentProcess,
            GetExitCodeProcess, InitializeProcThreadAttributeList, LPPROC_THREAD_ATTRIBUTE_LIST,
            PROC_THREAD_ATTRIBUTE_HANDLE_LIST, PROC_THREAD_ATTRIBUTE_JOB_LIST,
            PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE, PROCESS_INFORMATION, STARTF_USESTDHANDLES,
            STARTUPINFOEXW, UpdateProcThreadAttribute, WaitForSingleObject,
        },
    },
};

/// Observe this fixture process's terminal geometry in cells.
#[cfg(any(test, feature = "test-support"))]
pub fn current_terminal_size() -> io::Result<(u16, u16)> {
    let mut info = CONSOLE_SCREEN_BUFFER_INFO::default();
    // SAFETY: the process's console handle is queried into a valid output
    // structure. The API validates whether this process has a console.
    checked(unsafe { GetConsoleScreenBufferInfo(GetStdHandle(STD_OUTPUT_HANDLE), &mut info) })?;
    let cols = info.srWindow.Right - info.srWindow.Left + 1;
    let rows = info.srWindow.Bottom - info.srWindow.Top + 1;
    Ok((cols as u16, rows as u16))
}

/// A complete launch plan supplied by a trusted process-owning crate.
pub struct Launch<'a> {
    /// Resolved absolute executable path.
    pub executable: &'a Path,
    /// Literal argument values following the executable name.
    pub args: &'a [OsString],
    /// Working directory for the child.
    pub cwd: &'a Path,
    /// Exact environment, which is never extended from the harness.
    pub environment: &'a [(OsString, OsString)],
}

/// Find a Windows executable by the standard Rust launcher search order.
/// Relative PATH entries use the calling process's directory, before the
/// child working directory is applied. PTY callers use their explicit plan
/// search instead.
pub fn resolve_process_executable(
    file_name: &OsStr,
    supplied_path: Option<&OsStr>,
) -> io::Result<PathBuf> {
    let parent_cwd = std::env::current_dir()?;
    let child_path = supplied_path.into_iter().flat_map(std::env::split_paths);
    let standard_dirs = [
        std::env::current_exe()
            .ok()
            .and_then(|app| app.parent().map(Path::to_owned)),
        system_directory(GetSystemDirectoryW),
        system_directory(GetWindowsDirectoryW),
    ];
    let parent_path = std::env::var_os("PATH");
    let parent_dirs = parent_path
        .as_deref()
        .into_iter()
        .flat_map(std::env::split_paths);
    find_executable(
        &parent_cwd,
        file_name,
        child_path
            .chain(standard_dirs.into_iter().flatten())
            .chain(parent_dirs),
    )
    .ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "Windows executable was not found on the process search path",
        )
    })
}

fn find_executable(
    parent_cwd: &Path,
    file_name: &OsStr,
    dirs: impl Iterator<Item = PathBuf>,
) -> Option<PathBuf> {
    dirs.filter(|directory| !directory.as_os_str().is_empty())
        .map(|directory| parent_cwd.join(directory).join(file_name))
        .find(|candidate| candidate.is_file())
}

fn system_directory(get: unsafe extern "system" fn(*mut u16, u32) -> u32) -> Option<PathBuf> {
    let mut storage = vec![0; 260];
    // SAFETY: the native getter receives a writable array and its exact size.
    let mut length = unsafe { get(storage.as_mut_ptr(), storage.len() as u32) } as usize;
    if length >= storage.len() {
        storage.resize(length.checked_add(1)?, 0);
        // SAFETY: the resized array and its new capacity are supplied together.
        length = unsafe { get(storage.as_mut_ptr(), storage.len() as u32) } as usize;
    }
    if length == 0 || length >= storage.len() {
        return None;
    }
    Some(PathBuf::from(OsString::from_wide(&storage[..length])))
}

/// A live or exited child and its non-inheritable Job Object.
///
/// Closing this owner requests termination of every remaining member.
pub struct Child {
    job: OwnedHandle,
    process: OwnedHandle,
}

impl Child {
    /// Return the root exit code if the process handle is signaled.
    pub fn try_wait(&self) -> io::Result<Option<u32>> {
        let handle = self.process.as_raw_handle();
        // SAFETY: this object owns the process handle throughout both calls.
        match unsafe { WaitForSingleObject(handle, 0) } {
            WAIT_TIMEOUT => Ok(None),
            WAIT_OBJECT_0 => {
                let mut code = 0;
                // SAFETY: the output slot is writable and the handle is valid.
                checked(unsafe { GetExitCodeProcess(handle, &mut code) })?;
                Ok(Some(code))
            }
            _ => Err(io::Error::last_os_error()),
        }
    }

    /// Request forced termination of the job and all its members.
    pub fn terminate(&self) -> io::Result<()> {
        // SAFETY: the owned job handle is valid.
        checked(unsafe { TerminateJobObject(self.job.as_raw_handle(), 1) })
    }

    /// Read the operating system's current count of non-exited members.
    pub fn active_processes(&self) -> io::Result<u32> {
        let mut accounting = JOBOBJECT_BASIC_ACCOUNTING_INFORMATION::default();
        // SAFETY: the output slot is initialized, of the exact requested type
        // and size, and lives for the call. The return length is optional.
        checked(unsafe {
            QueryInformationJobObject(
                self.job.as_raw_handle(),
                JobObjectBasicAccountingInformation,
                (&raw mut accounting).cast(),
                size_of::<JOBOBJECT_BASIC_ACCOUNTING_INFORMATION>() as u32,
                null_mut(),
            )
        })?;
        Ok(accounting.ActiveProcesses)
    }
}

/// A rooted job with stdin closed and separate stdout and stderr readers.
pub struct Piped {
    /// Process and job ownership.
    pub child: Child,
    /// Standard output reader, to be drained independently.
    pub stdout: File,
    /// Standard error reader, to be drained independently.
    pub stderr: File,
}

/// Launch with closed stdin and piped output, without opening a console window.
pub fn spawn_piped(plan: Launch<'_>) -> io::Result<Piped> {
    let (stdout, child_stdout) = pipe()?;
    let (stderr, child_stderr) = pipe()?;
    let stdin = File::open("NUL")?;
    let stdin = inheritable(stdin.as_raw_handle())?;
    let child_stdout = inheritable(child_stdout.as_raw_handle())?;
    let child_stderr = inheritable(child_stderr.as_raw_handle())?;
    let handles = [
        stdin.as_raw_handle(),
        child_stdout.as_raw_handle(),
        child_stderr.as_raw_handle(),
    ];
    let child = spawn(plan, Stdio::Pipes(&handles))?;
    Ok(Piped {
        child,
        stdout,
        stderr,
    })
}

/// The owned native ConPTY handle.
///
/// Keep the output reader draining while closing this handle.
pub struct Console(HPCON);

impl Console {
    /// Resize the attached terminal using positive dimensions in cells.
    pub fn resize(&self, cols: u16, rows: u16) -> io::Result<()> {
        let size = dimensions(cols, rows)?;
        // SAFETY: this object uniquely owns the console handle.
        hresult(unsafe { ResizePseudoConsole(self.0, size) })
    }
}

impl Drop for Console {
    fn drop(&mut self) {
        // SAFETY: this object owns the console handle and drops it once.
        unsafe { ClosePseudoConsole(self.0) };
    }
}

/// The parts of a Windows terminal, whose output is drained separately.
pub struct Terminal {
    /// Root child and Job Object.
    pub child: Child,
    /// Connection used to send input bytes.
    pub input: File,
    /// Terminal's combined output reader.
    pub output: File,
    /// Handle used to resize and close the terminal.
    pub console: Console,
}

/// Create a pseudoconsole and launch into it and the Job Object in one call.
pub fn spawn_terminal(plan: Launch<'_>, cols: u16, rows: u16) -> io::Result<Terminal> {
    let size = dimensions(cols, rows)?;
    let (console_input, input) = pipe()?;
    let (output, console_output) = pipe()?;
    let mut handle = 0;
    // SAFETY: the pipe ends and output slot remain valid during the call.
    // Flags zero choose a fresh console with no cursor-inheritance exchange.
    hresult(unsafe {
        CreatePseudoConsole(
            size,
            console_input.as_raw_handle(),
            console_output.as_raw_handle(),
            0,
            &mut handle,
        )
    })?;
    let console = Console(handle);
    drop(console_input);
    drop(console_output);
    let child = spawn(plan, Stdio::Terminal(&console))?;
    Ok(Terminal {
        child,
        input,
        output,
        console,
    })
}

fn dimensions(cols: u16, rows: u16) -> io::Result<COORD> {
    let x = i16::try_from(cols).ok().filter(|&n| n > 0);
    let y = i16::try_from(rows).ok().filter(|&n| n > 0);
    match (x, y) {
        (Some(x), Some(y)) => Ok(COORD { X: x, Y: y }),
        _ => Err(invalid(
            "ConPTY size must be between 1 and 32767 cells on each axis",
        )),
    }
}

enum Stdio<'a> {
    Pipes(&'a [HANDLE; 3]),
    Terminal(&'a Console),
}

fn spawn(plan: Launch<'_>, stdio: Stdio<'_>) -> io::Result<Child> {
    if !plan.executable.is_absolute() {
        return Err(invalid(
            "Windows launch requires a resolved absolute executable path",
        ));
    }
    let (executable, mut command_line) = launch_arguments(plan.executable, plan.args)?;
    let cwd = wide_terminated(plan.cwd.as_os_str())?;
    let environment = environment_block(plan.environment)?;

    let job = job()?;
    let jobs = [job.as_raw_handle()];
    let mut attributes = Attributes::new()?;
    attributes.add(
        PROC_THREAD_ATTRIBUTE_JOB_LIST,
        jobs.as_ptr().cast(),
        size_of_val(&jobs),
    )?;
    let mut startup = STARTUPINFOEXW::default();
    let (inherit, flags) = match stdio {
        Stdio::Pipes(handles) => {
            attributes.add(
                PROC_THREAD_ATTRIBUTE_HANDLE_LIST,
                handles.as_ptr().cast(),
                size_of_val(handles),
            )?;
            startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
            [
                startup.StartupInfo.hStdInput,
                startup.StartupInfo.hStdOutput,
                startup.StartupInfo.hStdError,
            ] = *handles;
            (1, CREATE_NO_WINDOW)
        }
        Stdio::Terminal(console) => {
            attributes.add(
                PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE,
                console.0 as *const c_void,
                size_of::<HPCON>(),
            )?;
            // ConPTY supplies these handles. Inheriting the caller's redirected
            // stdio could send raw terminal output past the owned pipe.
            startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
            startup.StartupInfo.hStdInput = INVALID_HANDLE_VALUE;
            startup.StartupInfo.hStdOutput = INVALID_HANDLE_VALUE;
            startup.StartupInfo.hStdError = INVALID_HANDLE_VALUE;
            (0, 0)
        }
    };
    startup.lpAttributeList = attributes.as_ptr();
    startup.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
    let mut info = PROCESS_INFORMATION::default();
    // SAFETY: the strings and environment are terminated, and command_line
    // is writable. The initialized attribute list and its referenced handles
    // outlive this call. The startup size and creation flag describe the
    // enclosing extended structure. The output slot has the exact native type.
    checked(unsafe {
        CreateProcessW(
            executable.as_ptr(),
            command_line.as_mut_ptr(),
            null(),
            null(),
            inherit,
            flags | EXTENDED_STARTUPINFO_PRESENT | CREATE_UNICODE_ENVIRONMENT,
            environment.as_ptr().cast(),
            cwd.as_ptr(),
            &startup.StartupInfo,
            &mut info,
        )
    })?;
    // SAFETY: successful CreateProcessW transfers both distinct handles.
    let (process, _thread) = unsafe {
        (
            OwnedHandle::from_raw_handle(info.hProcess),
            OwnedHandle::from_raw_handle(info.hThread),
        )
    };
    Ok(Child { job, process })
}

fn job() -> io::Result<OwnedHandle> {
    // SAFETY: null attributes and name create an unnamed, non-inheritable job.
    let handle = unsafe { CreateJobObjectW(null(), null()) };
    if handle.is_null() {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: a non-null result transfers ownership of the new handle.
    let handle = unsafe { OwnedHandle::from_raw_handle(handle) };
    let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
    limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    // SAFETY: the initialized input layout and length match the information class.
    checked(unsafe {
        SetInformationJobObject(
            handle.as_raw_handle(),
            JobObjectExtendedLimitInformation,
            (&raw const limits).cast(),
            size_of_val(&limits) as u32,
        )
    })?;
    Ok(handle)
}

fn pipe() -> io::Result<(File, File)> {
    let mut reader = null_mut();
    let mut writer = null_mut();
    // SAFETY: both output slots are valid. Null security attributes create
    // non-inheritable endpoints.
    checked(unsafe { CreatePipe(&mut reader, &mut writer, null(), 0) })?;
    // SAFETY: success transfers both distinct handles.
    Ok(unsafe { (File::from_raw_handle(reader), File::from_raw_handle(writer)) })
}

fn inheritable(handle: HANDLE) -> io::Result<OwnedHandle> {
    let mut duplicate = null_mut();
    // SAFETY: the source is borrowed for this call. Both process pseudo-handles
    // refer to the current process and the output slot is valid.
    checked(unsafe {
        let owner = GetCurrentProcess();
        DuplicateHandle(
            owner,
            handle,
            owner,
            &mut duplicate,
            0,
            1,
            DUPLICATE_SAME_ACCESS,
        )
    })?;
    // SAFETY: success transfers a new and independent handle.
    Ok(unsafe { OwnedHandle::from_raw_handle(duplicate) })
}

struct Attributes(Box<[usize]>);

impl Attributes {
    fn new() -> io::Result<Self> {
        let mut bytes = 0;
        // SAFETY: null with a valid size pointer queries the required storage.
        let _queried = unsafe { InitializeProcThreadAttributeList(null_mut(), 2, 0, &mut bytes) };
        if bytes == 0 {
            return Err(io::Error::last_os_error());
        }
        let mut storage = vec![0_usize; bytes.div_ceil(size_of::<usize>())].into_boxed_slice();
        // SAFETY: pointer-sized storage is aligned and at least bytes long.
        checked(unsafe {
            InitializeProcThreadAttributeList(storage.as_mut_ptr().cast(), 2, 0, &mut bytes)
        })?;
        Ok(Self(storage))
    }

    fn as_ptr(&mut self) -> LPPROC_THREAD_ATTRIBUTE_LIST {
        self.0.as_mut_ptr().cast()
    }

    fn add(&mut self, attribute: u32, value: *const c_void, size: usize) -> io::Result<()> {
        // SAFETY: spawn supplies the documented input for each attribute. The
        // storage and every attribute value live through process creation.
        checked(unsafe {
            UpdateProcThreadAttribute(
                self.as_ptr(),
                0,
                attribute as usize,
                value,
                size,
                null_mut(),
                null(),
            )
        })
    }
}

impl Drop for Attributes {
    fn drop(&mut self) {
        // SAFETY: only successful initialization constructs this type.
        unsafe { DeleteProcThreadAttributeList(self.as_ptr()) };
    }
}

fn wide_terminated(value: &OsStr) -> io::Result<Vec<u16>> {
    let mut units = value.encode_wide().collect::<Vec<_>>();
    if units.contains(&0) {
        return Err(invalid("Windows spawn strings cannot contain NUL"));
    }
    units.push(0);
    Ok(units)
}

fn launch_arguments(executable: &Path, args: &[OsString]) -> io::Result<(Vec<u16>, Vec<u16>)> {
    if executable
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("cmd") || ext.eq_ignore_ascii_case("bat"))
    {
        return batch_arguments(executable, args);
    }
    let mut command = Vec::new();
    quote(&mut command, executable.as_os_str())?;
    for arg in args {
        command.push(b' ' as u16);
        quote(&mut command, arg)?;
    }
    command.push(0);
    Ok((wide_terminated(executable.as_os_str())?, command))
}

fn batch_arguments(script: &Path, args: &[OsString]) -> io::Result<(Vec<u16>, Vec<u16>)> {
    // CreateProcessW needs a real executable. Use the system interpreter,
    // never a COMSPEC supplied by the launch environment.
    let cmd = system_directory(GetSystemDirectoryW)
        .ok_or_else(|| {
            io::Error::other("the Windows system directory is unavailable for batch launch")
        })?
        .join("cmd.exe");
    let script = script.as_os_str();
    if script.as_encoded_bytes().starts_with(br"\\?\")
        || script.as_encoded_bytes().starts_with(br"\\.\")
    {
        return Err(invalid(
            "batch launch requires a conventional Windows path; use the underlying executable for device or verbatim paths",
        ));
    }
    let mut command: Vec<_> = "cmd.exe /d /s /e:on /v:off /c \"".encode_utf16().collect();
    batch_quote(&mut command, script, true)?;
    for arg in args {
        command.push(b' ' as u16);
        batch_quote(&mut command, arg, false)?;
    }
    command.extend([b'"' as u16, 0]);
    if command.len() > 8191 {
        return Err(invalid(
            "batch launch exceeds cmd.exe's argument limit; use the underlying executable",
        ));
    }
    Ok((wide_terminated(cmd.as_os_str())?, command))
}

fn batch_quote(out: &mut Vec<u16>, value: &OsStr, script: bool) -> io::Result<()> {
    // /s strips the one outer pair of quotes after /c. Every token keeps its
    // own pair. Refuse expansion, parser controls, and argument operators
    // rather than relying on an unknown script to escape a second parse.
    let units: Vec<_> = value.encode_wide().collect();
    if units.iter().any(|&unit| {
        unit < 32
            || [b'"', b'%', b'!']
                .into_iter()
                .any(|byte| unit == u16::from(byte))
            || (!script && b"&|<>^()".iter().any(|&byte| unit == u16::from(byte)))
    }) {
        return Err(invalid(
            "batch path or argument contains Windows command-interpreter syntax; use the underlying executable",
        ));
    }
    out.push(b'"' as u16);
    out.extend(units);
    out.push(b'"' as u16);
    Ok(())
}

fn quote(output: &mut Vec<u16>, value: &OsStr) -> io::Result<()> {
    const QUOTE: u16 = b'"' as u16;
    const SLASH: u16 = b'\\' as u16;
    output.push(QUOTE);
    let mut slashes = 0;
    for unit in value.encode_wide() {
        match unit {
            0 => return Err(invalid("Windows spawn arguments cannot contain NUL")),
            SLASH => slashes += 1,
            _ => {
                let escaped = if unit == QUOTE {
                    slashes * 2 + 1
                } else {
                    slashes
                };
                output.extend(std::iter::repeat_n(SLASH, escaped));
                output.push(unit);
                slashes = 0;
            }
        }
    }
    output.extend(std::iter::repeat_n(SLASH, slashes * 2));
    output.push(QUOTE);
    Ok(())
}

fn environment_block(vars: &[(OsString, OsString)]) -> io::Result<Vec<u16>> {
    let mut sorted = Vec::with_capacity(vars.len());
    // Stable sorting keeps the last declaration first among aliases.
    for (key, value) in vars.iter().rev() {
        if key.is_empty() || key.as_encoded_bytes().contains(&b'=') {
            return Err(invalid(
                "Windows spawn environment names must be nonempty and contain no equals sign",
            ));
        }
        sorted.push((wide_terminated(key)?, value));
    }
    sorted.sort_by(|(left, _), (right, _)| compare_environment_keys(left, right));
    sorted.dedup_by(|(left, _), (right, _)| compare_environment_keys(left, right).is_eq());
    let mut block = Vec::new();
    for (mut key, value) in sorted {
        *key.last_mut()
            .ok_or_else(|| invalid("empty environment name"))? = b'=' as u16;
        block.extend(key);
        block.extend(wide_terminated(value)?);
    }
    if block.is_empty() {
        block.push(0);
    }
    block.push(0);
    Ok(block)
}

fn compare_environment_keys(left: &[u16], right: &[u16]) -> Ordering {
    // SAFETY: both slices are valid UTF-16 storage ending in NUL. Windows
    // defines environment name order using case-insensitive ordinal comparison.
    match unsafe { CompareStringOrdinal(left.as_ptr(), -1, right.as_ptr(), -1, 1) } {
        CSTR_LESS_THAN => Ordering::Less,
        CSTR_EQUAL => Ordering::Equal,
        CSTR_GREATER_THAN => Ordering::Greater,
        _ => left.cmp(right),
    }
}

fn checked(result: i32) -> io::Result<()> {
    if result == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn hresult(result: i32) -> io::Result<()> {
    // ConPTY returns HRESULT_FROM_WIN32. Preserve the OS code where available.
    if result >= 0 {
        Ok(())
    } else if (result as u32 >> 16) & 0x1fff == 7 {
        Err(io::Error::from_raw_os_error(result & 0xffff))
    } else {
        Err(io::Error::other(format!("ConPTY HRESULT {result:#010x}")))
    }
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        env,
        io::{Read as _, Write as _},
        process::Command,
        sync::{
            Arc, Mutex,
            mpsc::{self, Receiver},
        },
        thread,
        time::{Duration, Instant},
    };
    use windows_sys::Win32::System::{
        JobObjects::IsProcessInJob,
        Threading::{
            GetProcessId, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
            WaitForMultipleObjects,
        },
    };

    const FIXTURE_DEPTH: &str = "EPH_JOB_FIXTURE_DEPTH";
    const FIXTURE: [&str; 3] = ["--exact", "windows::tests::job_fixture", "--nocapture"];
    const SIZE_FIXTURE: &str = "EPH_CONPTY_TEST_SIZE";
    const TREE_STARTUP: Duration = Duration::from_secs(30);
    const TREE_SHUTDOWN: Duration = Duration::from_secs(15);
    const READY_LINE: &str = "descendant fixture ready at depth ";

    #[test]
    fn environment_names_follow_windows_ordinal_order_and_last_alias_wins() {
        let encoded = environment_block(&[
            ("ENV_é".into(), "first".into()),
            ("Zebra".into(), "last key".into()),
            ("case".into(), "old case".into()),
            ("CASE".into(), "new case".into()),
            ("ENV_É".into(), "last alias".into()),
        ])
        .unwrap();
        assert_eq!(
            String::from_utf16(&encoded).unwrap(),
            "CASE=new case\0ENV_É=last alias\0Zebra=last key\0\0"
        );
    }

    #[test]
    fn geometry_fixture() {
        if env::var_os(SIZE_FIXTURE).is_none() {
            return;
        }
        let mut line = String::new();
        io::stdin().read_line(&mut line).unwrap();
        assert_eq!(line.trim(), "sample");
        let (cols, rows) = current_terminal_size().unwrap();
        println!("observed-geometry={cols}x{rows}");
    }

    #[test]
    #[expect(
        clippy::disallowed_methods,
        reason = "process: the fixture creates real descendants to test inherited Job Object membership"
    )]
    fn job_fixture() {
        let Some(depth) = env::var_os(FIXTURE_DEPTH) else {
            return;
        };
        let depth: u8 = depth.to_str().unwrap().parse().unwrap();
        let mut child = if depth < 2 {
            Some(
                Command::new(env::current_exe().unwrap())
                    .args(FIXTURE)
                    .env(FIXTURE_DEPTH, (depth + 1).to_string())
                    .spawn()
                    .unwrap(),
            )
        } else {
            None
        };
        println!("{READY_LINE}{depth} pid={};", std::process::id());
        io::stdout().flush().unwrap();
        // Keep the members available throughout startup and the shutdown
        // observation window. This fallback only bounds a failed fixture.
        thread::sleep(Duration::from_secs(60));
        if let Some(child) = &mut child {
            assert!(child.wait().unwrap().success());
        }
    }

    fn until(mut predicate: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !predicate() {
            assert!(
                Instant::now() < deadline,
                "job did not reach the expected state"
            );
            thread::sleep(Duration::from_millis(10));
        }
    }

    struct FixtureOutput {
        text: Arc<Mutex<String>>,
        eof: Receiver<io::Result<()>>,
    }

    impl FixtureOutput {
        fn collect(mut stream: impl io::Read + Send + 'static) -> Self {
            let text = Arc::new(Mutex::new(String::new()));
            let writer = Arc::clone(&text);
            let (sender, eof) = mpsc::channel();
            thread::spawn(move || {
                let result = (|| {
                    let mut buffer = [0; 512];
                    loop {
                        let count = stream.read(&mut buffer)?;
                        if count == 0 {
                            return Ok(());
                        }
                        writer
                            .lock()
                            .unwrap()
                            .push_str(&String::from_utf8_lossy(&buffer[..count]));
                    }
                })();
                let _reported = sender.send(result);
            });
            Self { text, eof }
        }

        fn snapshot(&self) -> String {
            self.text.lock().unwrap().clone()
        }

        fn expect_eof(&self) {
            assert!(
                matches!(self.eof.recv_timeout(TREE_SHUTDOWN), Ok(Ok(()))),
                "fixture stream did not reach EOF: {:?}",
                self.snapshot()
            );
        }
    }

    fn wait_for_live_tree(
        child: &Child,
        stdout: &FixtureOutput,
        stderr: Option<&FixtureOutput>,
    ) -> [OwnedHandle; 3] {
        let deadline = Instant::now() + TREE_STARTUP;
        let markers = [0, 1, 2].map(|depth| format!("{READY_LINE}{depth} pid="));
        loop {
            let active = child.active_processes().unwrap();
            let exit = child.try_wait().unwrap();
            let output = stdout.snapshot();
            let pids = markers.each_ref().map(|marker| {
                let (_, tail) = output.split_once(marker)?;
                let (pid, _) = tail.split_once(';')?;
                pid.parse::<u32>().ok()
            });
            if active >= 3
                && exit.is_none()
                && let [Some(root), Some(child_pid), Some(grandchild)] = pids
            {
                // SAFETY: the launcher retains a valid handle to the root.
                assert_eq!(root, unsafe { GetProcessId(child.process.as_raw_handle()) });
                assert!(root != child_pid && root != grandchild && child_pid != grandchild);
                return [root, child_pid, grandchild].map(|pid| live_job_member(child, pid));
            }
            assert!(
                exit.is_none() && Instant::now() < deadline,
                "fixture tree startup failed (active {active}, root exit {exit:?})\nstdout: {output:?}\nstderr: {:?}",
                stderr.map(FixtureOutput::snapshot)
            );
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn live_job_member(child: &Child, pid: u32) -> OwnedHandle {
        // SAFETY: the fixture supplied a PID and OpenProcess accepts these
        // query and wait rights. A non-null result transfers ownership.
        let raw = unsafe {
            OpenProcess(
                PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
                0,
                pid,
            )
        };
        assert!(
            !raw.is_null(),
            "open fixture PID {pid}: {}",
            io::Error::last_os_error()
        );
        // SAFETY: OpenProcess succeeded and returned an owned process handle.
        let process = unsafe { OwnedHandle::from_raw_handle(raw) };
        let mut included = 0;
        // SAFETY: both handles are valid and the output slot is writable.
        checked(unsafe {
            IsProcessInJob(
                process.as_raw_handle(),
                child.job.as_raw_handle(),
                &mut included,
            )
        })
        .unwrap();
        assert_ne!(included, 0, "fixture PID {pid} escaped the job");
        // SAFETY: the handle is valid and carries synchronization rights.
        assert_eq!(
            unsafe { WaitForSingleObject(process.as_raw_handle(), 0) },
            WAIT_TIMEOUT
        );
        process
    }

    fn terminate_fixture(child: &Child) {
        child.terminate().unwrap();
        let deadline = Instant::now() + TREE_SHUTDOWN;
        loop {
            let active = child.active_processes().unwrap();
            let exit = child.try_wait().unwrap();
            if active == 0 && exit.is_some() {
                assert_eq!(exit, Some(1));
                return;
            }
            assert!(
                Instant::now() < deadline,
                "terminated job still has {active} active members, root exit {exit:?}"
            );
            thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn piped_spawn_owns_its_root_and_both_generations() {
        let executable = env::current_exe().unwrap();
        let cwd = env::current_dir().unwrap();
        let args = FIXTURE.map(OsString::from);
        let environment = [(FIXTURE_DEPTH.into(), "0".into())];
        let Piped {
            child,
            stdout,
            stderr,
        } = spawn_piped(Launch {
            executable: &executable,
            args: &args,
            cwd: &cwd,
            environment: &environment,
        })
        .unwrap();
        let stdout = FixtureOutput::collect(stdout);
        let stderr = FixtureOutput::collect(stderr);
        let _members = wait_for_live_tree(&child, &stdout, Some(&stderr));
        terminate_fixture(&child);
        stdout.expect_eof();
        stderr.expect_eof();
    }

    #[test]
    fn closing_the_last_job_handle_without_explicit_kill_stops_all_members() {
        let executable = env::current_exe().unwrap();
        let cwd = env::current_dir().unwrap();
        let args = FIXTURE.map(OsString::from);
        let environment = [(FIXTURE_DEPTH.into(), "0".into())];
        let Piped {
            child,
            stdout,
            stderr,
        } = spawn_piped(Launch {
            executable: &executable,
            args: &args,
            cwd: &cwd,
            environment: &environment,
        })
        .unwrap();
        let stdout = FixtureOutput::collect(stdout);
        let stderr = FixtureOutput::collect(stderr);
        let members = wait_for_live_tree(&child, &stdout, Some(&stderr));
        drop(child);
        let handles = members.each_ref().map(AsRawHandle::as_raw_handle);
        // SAFETY: these valid and distinct process handles live through the
        // wait. They cannot keep the Job Object alive. Wait for all of them
        // within one bound that ends before the fixtures' own fallback.
        assert_eq!(
            unsafe {
                WaitForMultipleObjects(
                    handles.len() as u32,
                    handles.as_ptr(),
                    1,
                    TREE_SHUTDOWN.as_millis() as u32,
                )
            },
            WAIT_OBJECT_0
        );
        stdout.expect_eof();
        stderr.expect_eof();
    }

    #[test]
    fn the_same_native_launch_puts_the_pty_child_and_descendants_in_its_job() {
        let executable = env::current_exe().unwrap();
        let cwd = env::current_dir().unwrap();
        let args = FIXTURE.map(OsString::from);
        let environment = [(FIXTURE_DEPTH.into(), "0".into())];
        let Terminal {
            child,
            input,
            output,
            console,
        } = spawn_terminal(
            Launch {
                executable: &executable,
                args: &args,
                cwd: &cwd,
                environment: &environment,
            },
            80,
            24,
        )
        .unwrap();
        let stdout = FixtureOutput::collect(output);
        console.resize(100, 28).unwrap();
        let _members = wait_for_live_tree(&child, &stdout, None);
        terminate_fixture(&child);
        drop(input);
        drop(console);
        stdout.expect_eof();
    }

    #[test]
    fn rejects_invalid_launch_data_instead_of_truncating_it() {
        assert!(dimensions(0, 20).is_err());
        assert!(dimensions(10, u16::MAX).is_err());
        assert!(wide_terminated(OsStr::new("broken\0path")).is_err());
        assert!(quote(&mut Vec::new(), OsStr::new("broken\0arg")).is_err());
        assert!(environment_block(&[("name=injected".into(), "value".into())]).is_err());
        assert_eq!(environment_block(&[]).unwrap(), [0, 0]);
    }

    #[test]
    fn native_geometry_accepts_the_maximum_and_rejects_each_invalid_axis() {
        for (cols, rows) in [(1, 1), (32767, 32767)] {
            let result = dimensions(cols, rows).unwrap();
            assert_eq!((result.X, result.Y), (cols as i16, rows as i16));
        }
        for (cols, rows) in [(0, 1), (1, 0), (32768, 1), (1, 32768)] {
            assert_eq!(
                dimensions(cols, rows).err().unwrap().kind(),
                io::ErrorKind::InvalidInput
            );
        }
    }

    #[test]
    fn process_search_resolves_a_real_supplied_path_and_refuses_an_absent_program() {
        let executable = env::current_exe().unwrap();
        let bin = executable.parent().unwrap();
        let name = executable.file_name().unwrap();
        assert_eq!(
            resolve_process_executable(name, Some(bin.as_os_str())).unwrap(),
            executable
        );
        assert_eq!(
            resolve_process_executable(
                OsStr::new("ephesus-absent-fixture-47c895.exe"),
                Some(bin.as_os_str())
            )
            .unwrap_err()
            .kind(),
            io::ErrorKind::NotFound
        );
    }

    #[test]
    fn system_paths_grow_their_buffer_and_refuse_incomplete_results() {
        unsafe extern "system" fn long_path(out: *mut u16, size: u32) -> u32 {
            if size < 301 {
                return 301;
            }
            for index in 0..300 {
                // SAFETY: the reported capacity covers these 300 units and NUL.
                unsafe {
                    out.add(index).write(u16::from(b'x'));
                }
            }
            // SAFETY: the supplied capacity has room for the terminator.
            unsafe {
                out.add(300).write(0);
            }
            300
        }
        unsafe extern "system" fn unavailable(_: *mut u16, _: u32) -> u32 {
            0
        }
        unsafe extern "system" fn incomplete(_: *mut u16, size: u32) -> u32 {
            size + 1
        }

        assert!(system_directory(GetSystemDirectoryW).unwrap().is_dir());
        assert!(system_directory(GetWindowsDirectoryW).unwrap().is_dir());
        assert_eq!(
            system_directory(long_path).unwrap(),
            PathBuf::from("x".repeat(300))
        );
        assert_eq!(system_directory(unavailable), None);
        assert_eq!(system_directory(incomplete), None);
    }

    #[test]
    fn native_arguments_preserve_quotes_and_trailing_backslashes() {
        for (input, expected) in [
            ("", "\"\""),
            ("plain", "\"plain\""),
            ("a\\b", "\"a\\b\""),
            ("a\"b", "\"a\\\"b\""),
            ("a\\", "\"a\\\\\""),
            ("a\\\"b", "\"a\\\\\\\"b\""),
        ] {
            let mut encoded = Vec::new();
            quote(&mut encoded, OsStr::new(input)).unwrap();
            assert_eq!(String::from_utf16(&encoded).unwrap(), expected, "{input:?}");
        }
    }

    #[test]
    fn batch_arguments_and_error_codes_preserve_the_checked_contract() {
        for extension in ["cmd", "bat"] {
            let path = PathBuf::from(format!("C:\\fixture dir\\tool.{extension}"));
            let (program, command) = launch_arguments(&path, &["arg with space".into()]).unwrap();
            assert!(
                OsString::from_wide(&program[..program.len() - 1])
                    .to_string_lossy()
                    .ends_with("cmd.exe")
            );
            let command = String::from_utf16(&command[..command.len() - 1]).unwrap();
            assert!(command.contains("cmd.exe /d /s /e:on /v:off /c"));
            assert!(command.contains("\"arg with space\""));
            let overhead = batch_arguments(&path, &["".into()]).unwrap().1.len();
            let largest = "x".repeat(8191 - overhead);
            assert_eq!(
                batch_arguments(&path, &[largest.clone().into()])
                    .unwrap()
                    .1
                    .len(),
                8191
            );
            assert!(batch_arguments(&path, &[format!("{largest}x").into()]).is_err());
            assert!(batch_arguments(&path, &["x".repeat(8192).into()]).is_err());
            for prefix in [r"\\?\C:\", r"\\.\C:\"] {
                let device = PathBuf::from(format!("{prefix}tool.{extension}"));
                assert_eq!(
                    batch_arguments(&device, &[]).unwrap_err().kind(),
                    io::ErrorKind::InvalidInput
                );
            }
        }
        for input in [
            "\0", "\n", "\"", "%value%", "!", "^", "&", "|", "<", ">", "(", ")",
        ] {
            assert!(
                batch_quote(&mut Vec::new(), OsStr::new(input), false).is_err(),
                "{input:?}"
            );
        }
        let mut normal = Vec::new();
        batch_quote(&mut normal, OsStr::new("a value\\with\\slashes"), false).unwrap();
        assert_eq!(
            String::from_utf16(&normal).unwrap(),
            "\"a value\\with\\slashes\""
        );
        checked(1).unwrap();
        // SAFETY: SetLastError only sets this test thread's error slot.
        unsafe {
            windows_sys::Win32::Foundation::SetLastError(5);
        }
        assert_eq!(checked(0).unwrap_err().raw_os_error(), Some(5));
        for success in [0, 1] {
            hresult(success).unwrap();
        }
        assert_eq!(
            hresult(0x8007_0005_u32 as i32).unwrap_err().raw_os_error(),
            Some(5)
        );
        assert!(
            hresult(0x8000_4005_u32 as i32)
                .unwrap_err()
                .to_string()
                .contains("0x80004005")
        );
        assert_eq!(
            compare_environment_keys(
                &wide_terminated(OsStr::new("Z")).unwrap(),
                &wide_terminated(OsStr::new("a")).unwrap()
            ),
            Ordering::Greater
        );
    }

    #[test]
    fn closing_the_console_ends_attached_clients_and_the_output_stream() {
        let executable = env::current_exe().unwrap();
        let cwd = env::current_dir().unwrap();
        let args = FIXTURE.map(OsString::from);
        let environment = [(FIXTURE_DEPTH.into(), "0".into())];
        let Terminal {
            child,
            input,
            output,
            console,
        } = spawn_terminal(
            Launch {
                executable: &executable,
                args: &args,
                cwd: &cwd,
                environment: &environment,
            },
            80,
            24,
        )
        .unwrap();
        let stdout = FixtureOutput::collect(output);
        let _members = wait_for_live_tree(&child, &stdout, None);
        drop(console);
        drop(input);
        until(|| child.try_wait().unwrap().is_some());
        stdout.expect_eof();
    }

    #[test]
    fn relative_process_search_entries_use_the_parent_directory() {
        let executable = env::current_exe().unwrap();
        let bin = executable.parent().unwrap();
        let parent = bin.parent().unwrap();
        let relative = bin.strip_prefix(parent).unwrap().to_owned();
        let file = executable.file_name().unwrap();
        let dirs = [PathBuf::new(), relative];
        assert_eq!(
            find_executable(parent, file, dirs.clone().into_iter()),
            Some(executable.clone())
        );
        assert_eq!(
            find_executable(
                &parent.join("different-child-directory"),
                file,
                dirs.into_iter()
            ),
            None
        );
    }

    #[test]
    fn conpty_child_observes_the_requested_size_after_resize() {
        let executable = env::current_exe().unwrap();
        let cwd = env::current_dir().unwrap();
        let args =
            ["--exact", "windows::tests::geometry_fixture", "--nocapture"].map(OsString::from);
        let mut environment = vec![(SIZE_FIXTURE.into(), "1".into())];
        if let Some(profile) = env::var_os("LLVM_PROFILE_FILE") {
            environment.push(("LLVM_PROFILE_FILE".into(), profile));
        }
        let Terminal {
            child,
            mut input,
            mut output,
            console,
        } = spawn_terminal(
            Launch {
                executable: &executable,
                args: &args,
                cwd: &cwd,
                environment: &environment,
            },
            80,
            24,
        )
        .unwrap();
        let drain = thread::spawn(move || {
            let mut text = String::new();
            output.read_to_string(&mut text).unwrap();
            text
        });
        console.resize(97, 29).unwrap();
        input.write_all(b"sample\r").unwrap();
        until(|| child.try_wait().unwrap().is_some());
        assert_eq!(child.try_wait().unwrap(), Some(0));
        drop(input);
        drop(console);
        let text = drain.join().unwrap();
        assert!(text.contains("observed-geometry=97x29"), "{text:?}");
    }
}

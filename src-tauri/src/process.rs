use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use tauri::{AppHandle, Emitter};

use crate::types::{EnvVar, LogLine, LogPayload, ProcessState, StatusPayload, Stream, process_key};
use crate::util::{UrlConfidence, detect_url, strip_ansi};

/// Key of the command currently open in the log panel (see `AppState::log_viewer`).
pub type LogViewer = Arc<Mutex<Option<String>>>;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(windows)]
pub const CREATE_NO_WINDOW_FLAG: u32 = 0x08000000;

// ── Windows Job Object ─────────────────────────────
// Ensures all child processes die when OneRun exits, even on crash/force-kill.

#[cfg(windows)]
static JOB_HANDLE: std::sync::OnceLock<usize> = std::sync::OnceLock::new();

#[cfg(windows)]
pub fn init_job_object() {
    extern "system" {
        fn CreateJobObjectW(attrs: *const u8, name: *const u16) -> *mut std::ffi::c_void;
        fn SetInformationJobObject(job: *mut std::ffi::c_void, class: u32, info: *const u8, len: u32) -> i32;
    }

    unsafe {
        let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
        if job.is_null() { return; }

        // JOBOBJECT_EXTENDED_LIMIT_INFORMATION: 144 bytes on 64-bit, 112 on 32-bit
        // LimitFlags offset is 16 bytes in, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE = 0x2000
        let mut info = [0u8; 144];
        let info_len: u32 = if std::mem::size_of::<usize>() == 8 { 144 } else { 112 };
        let flags_ptr = info.as_mut_ptr().add(16) as *mut u32;
        *flags_ptr = 0x2000; // JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE

        // JobObjectExtendedLimitInformation = 9
        SetInformationJobObject(job, 9, info.as_ptr(), info_len);

        let _ = JOB_HANDLE.set(job as usize);
    }
}

#[cfg(windows)]
fn assign_to_job(pid: u32) {
    extern "system" {
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut std::ffi::c_void;
        fn AssignProcessToJobObject(job: *mut std::ffi::c_void, proc: *mut std::ffi::c_void) -> i32;
        fn CloseHandle(handle: *mut std::ffi::c_void) -> i32;
    }

    if let Some(&job) = JOB_HANDLE.get() {
        unsafe {
            let proc = OpenProcess(0x001F0FFF, 0, pid);
            if !proc.is_null() {
                AssignProcessToJobObject(job as *mut std::ffi::c_void, proc);
                CloseHandle(proc);
            }
        }
    }
}

#[cfg(not(windows))]
pub fn init_job_object() {}

#[cfg(not(windows))]
fn assign_to_job(_pid: u32) {}

// ── Per-process Job Objects ───────────────────────
// Each spawned process gets its own job object so TerminateJobObject
// reliably kills ALL descendants (vite, node, etc.), unlike taskkill /T
// which misses detached children.

#[cfg(windows)]
fn create_process_job() -> Option<usize> {
    extern "system" {
        fn CreateJobObjectW(attrs: *const u8, name: *const u16) -> *mut std::ffi::c_void;
        fn SetInformationJobObject(job: *mut std::ffi::c_void, class: u32, info: *const u8, len: u32) -> i32;
    }
    unsafe {
        let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
        if job.is_null() { return None; }
        let mut info = [0u8; 144];
        let info_len: u32 = if std::mem::size_of::<usize>() == 8 { 144 } else { 112 };
        let flags_ptr = info.as_mut_ptr().add(16) as *mut u32;
        *flags_ptr = 0x2000; // JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
        SetInformationJobObject(job, 9, info.as_ptr(), info_len);
        Some(job as usize)
    }
}

#[cfg(not(windows))]
fn create_process_job() -> Option<usize> { None }

#[cfg(windows)]
fn assign_pid_to_job(job_handle: usize, pid: u32) {
    extern "system" {
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut std::ffi::c_void;
        fn AssignProcessToJobObject(job: *mut std::ffi::c_void, proc: *mut std::ffi::c_void) -> i32;
        fn CloseHandle(handle: *mut std::ffi::c_void) -> i32;
    }
    unsafe {
        let proc = OpenProcess(0x001F0FFF, 0, pid);
        if !proc.is_null() {
            AssignProcessToJobObject(job_handle as *mut std::ffi::c_void, proc);
            CloseHandle(proc);
        }
    }
}

#[cfg(not(windows))]
fn assign_pid_to_job(_job_handle: usize, _pid: u32) {}

#[cfg(windows)]
fn terminate_job(job_handle: usize) {
    extern "system" {
        fn TerminateJobObject(job: *mut std::ffi::c_void, exit_code: u32) -> i32;
        fn CloseHandle(handle: *mut std::ffi::c_void) -> i32;
    }
    unsafe {
        TerminateJobObject(job_handle as *mut std::ffi::c_void, 1);
        CloseHandle(job_handle as *mut std::ffi::c_void);
    }
}

#[cfg(not(windows))]
fn terminate_job(_job_handle: usize) {}

const MAX_LOG_LINES: usize = 2000;
/// Hard cap on the buffered log text per command. Line count alone is not
/// enough — a few thousand very long lines (stack traces, minified bundles)
/// would otherwise sit in memory for the lifetime of the app.
const MAX_LOG_BYTES: usize = 512 * 1024;
/// Longest single line we will buffer. Tools that draw progress with `\r`
/// and never emit `\n` would otherwise grow one line without bound.
const MAX_LINE_BYTES: usize = 8 * 1024;

// ── Shell helpers ──────────────────────────────────

#[cfg(windows)]
pub fn spawn_shell(command: &str, cwd: &str, env: &[EnvVar]) -> Result<std::process::Child, String> {
    let mut cmd = Command::new("cmd");
    cmd.args(["/C", command])
        .current_dir(cwd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .creation_flags(CREATE_NO_WINDOW_FLAG)
        .env("FORCE_COLOR", "3");
    for e in env {
        cmd.env(&e.key, &e.value);
    }
    cmd.spawn().map_err(|e| e.to_string())
}

#[cfg(not(windows))]
pub fn spawn_shell(command: &str, cwd: &str, env: &[EnvVar]) -> Result<std::process::Child, String> {
    use std::os::unix::process::CommandExt;
    let mut cmd = Command::new("sh");
    cmd.args(["-c", command])
        .current_dir(cwd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("FORCE_COLOR", "3");
    for e in env {
        cmd.env(&e.key, &e.value);
    }
    unsafe {
        cmd.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    cmd.spawn().map_err(|e| e.to_string())
}

#[cfg(windows)]
pub fn kill_tree(pid: u32) {
    let _ = Command::new("taskkill")
        .args(["/T", "/F", "/PID", &pid.to_string()])
        .creation_flags(CREATE_NO_WINDOW_FLAG)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .and_then(|mut c| c.wait());
}

#[cfg(not(windows))]
pub fn kill_tree(pid: u32) {
    // Kill the entire process group (negative PID) thanks to setsid in spawn.
    // stop() blocks on this before telling the UI the port is free, so poll
    // for the group to die instead of sleeping a flat two seconds.
    let group = -(pid as i32);
    unsafe { libc::kill(group, libc::SIGTERM) };

    for _ in 0..80 {
        std::thread::sleep(std::time::Duration::from_millis(25));
        // ESRCH — no process left in the group
        if unsafe { libc::kill(group, 0) } != 0 {
            return;
        }
    }
    unsafe { libc::kill(group, libc::SIGKILL) };
}

// ── Log buffer ─────────────────────────────────────

/// Push a log line. Returns `false` if the epoch no longer matches (stale reader).
fn push_log(
    procs: &Arc<Mutex<HashMap<String, ProcessState>>>,
    key: &str,
    text: String,
    stream: Stream,
    epoch: u64,
) -> bool {
    if let Ok(mut map) = procs.lock() {
        if let Some(ps) = map.get_mut(key) {
            if ps.epoch != epoch {
                return false; // stale reader — caller should stop
            }
            ps.log_bytes += text.len();
            ps.logs.push_back(LogLine { text, stream });
            while ps.logs.len() > MAX_LOG_LINES || ps.log_bytes > MAX_LOG_BYTES {
                match ps.logs.pop_front() {
                    Some(dropped) => ps.log_bytes -= dropped.text.len(),
                    None => break,
                }
            }
        }
    }
    true
}

// ── Stream reader ──────────────────────────────────

/// Spawns a thread that reads lines from a stream, stores them in the
/// process log buffer, and emits them as Tauri events.
fn spawn_reader(
    stream: impl std::io::Read + Send + 'static,
    stream_name: Stream,
    key: String,
    id: String,
    label: String,
    app: AppHandle,
    procs: Arc<Mutex<HashMap<String, ProcessState>>>,
    viewer: LogViewer,
    detect_urls: bool,
    epoch: u64,
) {
    thread::spawn(move || {
        // Handle one complete line. Returns false when this reader is stale
        // and should stop.
        let handle = |line: String| -> bool {
            // URL detection is the only reason to strip ANSI, and only lines
            // that mention a URL can match — skip the allocation otherwise.
            if detect_urls && line.contains("http") {
                let clean = strip_ansi(&line);
                if let Some((url, confidence)) = detect_url(&clean) {
                    // Decide under the lock, emit after releasing it — other
                    // readers and status queries contend on this mutex.
                    let mut changed = None;
                    if let Ok(mut map) = procs.lock() {
                        if let Some(ps) = map.get_mut(&key) {
                            // Stop if the process is gone or a newer run owns the slot
                            if !ps.running || ps.epoch != epoch {
                                return false;
                            }
                            let dominated =
                                ps.detected_url.is_some() && confidence < ps.url_confidence;
                            let unchanged = ps.detected_url.as_ref() == Some(&url);
                            if !dominated && !unchanged {
                                ps.detected_url = Some(url.clone());
                                ps.url_confidence = confidence;
                                changed = Some(url);
                            }
                        }
                    }
                    if let Some(url) = changed {
                        let _ = app.emit(
                            "process-status",
                            StatusPayload {
                                id: id.clone(),
                                label: label.clone(),
                                running: true,
                                url: Some(url),
                            },
                        );
                    }
                }
            }

            // Only the command open in the log panel needs live events; the
            // rest is read back from the buffer when the user opens it.
            let watched = viewer
                .lock()
                .ok()
                .is_some_and(|v| v.as_deref() == Some(key.as_str()));

            if watched {
                let _ = app.emit(
                    "process-log",
                    LogPayload {
                        id: id.clone(),
                        label: label.clone(),
                        text: line.clone(),
                        stream: stream_name,
                    },
                );
            }

            // push_log returns false if the epoch changed (stale reader)
            push_log(&procs, &key, line, stream_name, epoch)
        };

        read_lines(stream, handle);
    });
}

/// Read `stream` line by line, calling `on_line` for each one. Stops early
/// when `on_line` returns false.
///
/// Unlike `BufRead::lines()` this breaks on `\r` as well as `\n` and caps
/// line length at `MAX_LINE_BYTES`: progress-bar output (`docker pull`,
/// bundler spinners) is one endless `\r`-delimited "line" that would
/// otherwise grow in memory until the process exits.
fn read_lines(stream: impl std::io::Read, mut on_line: impl FnMut(String) -> bool) {
    let mut reader = BufReader::new(stream);
    let mut buf: Vec<u8> = Vec::with_capacity(256);
    let mut pending_cr = false; // last byte was '\r'
    let mut cr_flushed = false; // ...and it ended a non-empty line
    let mut stopped = false;

    'outer: loop {
        let mut lines: Vec<String> = Vec::new();

        let consumed = {
            let chunk = match reader.fill_buf() {
                Ok(c) => c,
                Err(_) => break,
            };
            if chunk.is_empty() {
                break; // EOF
            }
            for &b in chunk {
                match b {
                    b'\r' => {
                        cr_flushed = !buf.is_empty();
                        if cr_flushed {
                            lines.push(take_line(&mut buf));
                        }
                        pending_cr = true;
                    }
                    b'\n' => {
                        // CRLF: the '\r' already ended this line
                        if pending_cr && cr_flushed {
                            pending_cr = false;
                            continue;
                        }
                        pending_cr = false;
                        lines.push(take_line(&mut buf)); // may be empty — a real blank line
                    }
                    _ => {
                        pending_cr = false;
                        if buf.len() >= MAX_LINE_BYTES {
                            lines.push(take_line(&mut buf));
                        }
                        buf.push(b);
                    }
                }
            }
            chunk.len()
        };
        reader.consume(consumed);

        for line in lines {
            if !on_line(line) {
                stopped = true;
                break 'outer;
            }
        }
    }

    // Flush whatever was buffered when the stream ended
    if !stopped && !buf.is_empty() {
        on_line(take_line(&mut buf));
    }
}

fn take_line(buf: &mut Vec<u8>) -> String {
    let line = String::from_utf8_lossy(buf).into_owned();
    buf.clear();
    line
}

// ── Process lifecycle ──────────────────────────────

/// Start a shell process, wire up log streaming, and track it in state.
pub fn start(
    id: String,
    label: String,
    command: String,
    cwd: String,
    env: Vec<EnvVar>,
    app: AppHandle,
    processes: Arc<Mutex<HashMap<String, ProcessState>>>,
    viewer: LogViewer,
) -> Result<(), String> {
    let key = process_key(&id, &label);

    // Mark as running (only checks THIS command slot, not other commands)
    let epoch;
    {
        let mut map = processes.lock().map_err(|e| e.to_string())?;
        if let Some(ps) = map.get(&key) {
            if ps.running {
                return Err("Already running".into());
            }
        }
        let ps = map.entry(key.clone()).or_default();
        ps.logs.clear();
        ps.logs.shrink_to_fit();
        ps.log_bytes = 0;
        ps.running = true;
        ps.detected_url = None;
        ps.url_confidence = UrlConfidence::Normal;
        ps.epoch += 1;
        epoch = ps.epoch;
    }

    // Spawn
    let mut child = match spawn_shell(&command, &cwd, &env) {
        Ok(c) => c,
        Err(e) => {
            let mut map = processes.lock().unwrap();
            if let Some(ps) = map.get_mut(&key) {
                ps.running = false;
            }
            let _ = app.emit(
                "process-status",
                StatusPayload {
                    id,
                    label,
                    running: false,
                    url: None,
                },
            );
            return Err(e);
        }
    };

    // Store PID and assign to job objects
    let pid = child.id();
    assign_to_job(pid);  // global job: auto-kill on app crash

    // Per-process job: reliable kill of ALL descendants on stop
    let proc_job = create_process_job();
    if let Some(jh) = proc_job {
        assign_pid_to_job(jh, pid);
    }

    // Race check: a concurrent stop() can land between our epoch bump and this
    // pid write — it would see running=true but pid+jh both None, flip
    // running=false, and bump the epoch. Without this guard the child we just
    // spawned would have nothing tracking it (orphan port-holder).
    let race_lost = {
        let mut map = processes.lock().unwrap();
        match map.get_mut(&key) {
            Some(ps) if ps.epoch == epoch && ps.running => {
                ps.pid = Some(pid);
                ps.job_handle = proc_job;
                false
            }
            _ => true,
        }
    };

    if race_lost {
        if let Some(jh) = proc_job {
            terminate_job(jh);
        }
        kill_tree(pid);
        // Reap so the dead child doesn't linger as a zombie on Unix.
        thread::spawn(move || { let _ = child.wait(); });
        return Err("Start cancelled by concurrent stop".into());
    }

    let _ = app.emit(
        "process-status",
        StatusPayload {
            id: id.clone(),
            label: label.clone(),
            running: true,
            url: None,
        },
    );

    // Wire stdout
    if let Some(stdout) = child.stdout.take() {
        spawn_reader(
            stdout, Stream::Stdout,
            key.clone(), id.clone(), label.clone(),
            app.clone(), processes.clone(), viewer.clone(), true, epoch,
        );
    }

    // Wire stderr
    if let Some(stderr) = child.stderr.take() {
        spawn_reader(
            stderr, Stream::Stderr,
            key.clone(), id.clone(), label.clone(),
            app.clone(), processes.clone(), viewer, true, epoch,
        );
    }

    // Wait for exit
    let key_c = key;
    let id_c = id;
    let label_c = label;
    let app_c = app;
    let procs_c = processes;
    thread::spawn(move || {
        let _ = child.wait();
        {
            let mut map = procs_c.lock().unwrap();
            if let Some(ps) = map.get_mut(&key_c) {
                // Only clean up if this is still OUR process.
                // A newer start() would have bumped the epoch, meaning
                // this wait thread is stale and must not touch the state.
                if ps.epoch != epoch {
                    return; // stale — a newer process owns this slot
                }
                ps.running = false;
                ps.pid = None;
                ps.detected_url = None;
                // Kill any orphaned children and close the job handle
                if let Some(jh) = ps.job_handle.take() {
                    terminate_job(jh);
                }
            }
        }
        let _ = app_c.emit(
            "process-status",
            StatusPayload {
                id: id_c,
                label: label_c,
                running: false,
                url: None,
            },
        );
    });

    Ok(())
}

/// Stop a specific command by project id + label.
pub fn stop(
    id: &str,
    label: &str,
    processes: &Arc<Mutex<HashMap<String, ProcessState>>>,
    app: &AppHandle,
) -> Result<(), String> {
    let key = process_key(id, label);

    // Phase 1: flip state under lock and extract kill targets. We DO NOT emit
    // the status event yet — the UI must keep showing "Stop" until the process
    // is actually dead and the port is released, otherwise the user can click
    // Start before the OS releases the listening socket.
    let (jh, pid) = {
        let mut map = processes.lock().map_err(|e| e.to_string())?;
        let ps = map.get_mut(&key).filter(|ps| ps.running)
            .ok_or_else(|| "Not running".to_string())?;
        ps.running = false;
        ps.detected_url = None;
        ps.epoch += 1;
        let result = (ps.job_handle.take(), ps.pid.take());
        if result.0.is_none() && result.1.is_none() {
            return Err("Not running".into());
        }
        result
    }; // lock released

    // Phase 2: kill outside the lock (kill_tree spawns taskkill and waits)
    if let Some(jh) = jh {
        terminate_job(jh);
    }
    if let Some(pid) = pid {
        kill_tree(pid);
    }

    // Phase 3: now emit "running:false" — the process is dead, port is released,
    // so it's safe for the UI to flip the button to "Start".
    let _ = app.emit(
        "process-status",
        StatusPayload {
            id: id.to_string(),
            label: label.to_string(),
            running: false,
            url: None,
        },
    );
    Ok(())
}

/// Stop ALL running commands for a project.
pub fn stop_all(
    id: &str,
    processes: &Arc<Mutex<HashMap<String, ProcessState>>>,
    app: &AppHandle,
) -> Result<(), String> {
    let prefix = format!("{}::", id);

    // Phase 1: flip state under lock, collect kill targets and labels for later
    // emit. Status events are deferred until after the kill so the UI button
    // doesn't flip to "Start" before the port is actually released.
    let mut kill_targets: Vec<(String, Option<usize>, Option<u32>)> = Vec::new();
    {
        let mut map = processes.lock().map_err(|e| e.to_string())?;
        for (key, ps) in map.iter_mut() {
            if key.starts_with(&prefix) && ps.running {
                ps.running = false;
                ps.detected_url = None;
                ps.epoch += 1;
                let jh = ps.job_handle.take();
                let pid = ps.pid.take();
                let label = key.strip_prefix(&prefix).unwrap_or("").to_string();
                kill_targets.push((label, jh, pid));
            }
        }
    } // lock released

    if kill_targets.is_empty() {
        return Err("Nothing running".into());
    }

    // Phase 2: kill outside the lock
    for (_, jh, pid) in &kill_targets {
        if let Some(jh) = jh {
            terminate_job(*jh);
        }
        if let Some(pid) = pid {
            kill_tree(*pid);
        }
    }

    // Phase 3: now that processes are dead and ports released, emit status
    for (label, _, _) in kill_targets {
        let _ = app.emit(
            "process-status",
            StatusPayload {
                id: id.to_string(),
                label,
                running: false,
                url: None,
            },
        );
    }
    Ok(())
}

/// Kill all tracked processes (used on app shutdown).
pub fn kill_all(processes: &Arc<Mutex<HashMap<String, ProcessState>>>) {
    if let Ok(mut map) = processes.lock() {
        for ps in map.values_mut() {
            // Belt-and-suspenders: try both kill methods
            if let Some(jh) = ps.job_handle.take() {
                terminate_job(jh);
            }
            if let Some(pid) = ps.pid.take() {
                kill_tree(pid);
            }
        }
    }
}

/// Remove all process entries for a project (used when project is deleted).
pub fn purge_project(id: &str, processes: &Arc<Mutex<HashMap<String, ProcessState>>>) {
    let prefix = format!("{}::", id);

    // The UI stops a project before purging it, but if that failed we would
    // drop the pid and job handle here — leaking a kernel handle and leaving
    // an orphan process holding its port with no way left to stop it.
    let mut kill_targets: Vec<(Option<usize>, Option<u32>)> = Vec::new();
    if let Ok(mut map) = processes.lock() {
        map.retain(|key, ps| {
            if key.starts_with(&prefix) {
                if ps.job_handle.is_some() || ps.pid.is_some() {
                    kill_targets.push((ps.job_handle.take(), ps.pid.take()));
                }
                false
            } else {
                true
            }
        });
    } // lock released — kill_tree blocks

    for (jh, pid) in kill_targets {
        if let Some(jh) = jh {
            terminate_job(jh);
        }
        if let Some(pid) = pid {
            kill_tree(pid);
        }
    }
}

// ── Tests ──────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn collect(input: &[u8]) -> Vec<String> {
        let mut out = Vec::new();
        read_lines(input, |line| {
            out.push(line);
            true
        });
        out
    }

    #[test]
    fn splits_on_lf() {
        assert_eq!(collect(b"a\nb\nc\n"), vec!["a", "b", "c"]);
    }

    #[test]
    fn crlf_is_one_break() {
        assert_eq!(collect(b"a\r\nb\r\n"), vec!["a", "b"]);
    }

    #[test]
    fn keeps_blank_lines() {
        assert_eq!(collect(b"a\n\nb\n"), vec!["a", "", "b"]);
        assert_eq!(collect(b"a\r\n\r\nb\r\n"), vec!["a", "", "b"]);
    }

    #[test]
    fn splits_progress_bar_on_cr() {
        // A \r-only spinner used to accumulate as one endless line.
        assert_eq!(collect(b"\r10%\r20%\r30%\n"), vec!["10%", "20%", "30%"]);
    }

    #[test]
    fn flushes_trailing_partial_line() {
        assert_eq!(collect(b"a\nno newline"), vec!["a", "no newline"]);
    }

    #[test]
    fn caps_line_length() {
        let mut input = vec![b'x'; MAX_LINE_BYTES * 2 + 10];
        input.push(b'\n');
        let lines = collect(&input);
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0].len(), MAX_LINE_BYTES);
        assert_eq!(lines[1].len(), MAX_LINE_BYTES);
        assert_eq!(lines[2].len(), 10);
    }

    #[test]
    fn stops_when_handler_returns_false() {
        let mut seen = Vec::new();
        read_lines(&b"a\nb\nc\npartial"[..], |line| {
            let keep = line != "b";
            seen.push(line);
            keep
        });
        // Stops at "b" — no later lines, and no trailing flush.
        assert_eq!(seen, vec!["a", "b"]);
    }

    #[test]
    fn invalid_utf8_does_not_panic() {
        assert_eq!(collect(b"ok\n\xff\xfe\n"), vec!["ok", "\u{fffd}\u{fffd}"]);
    }
}

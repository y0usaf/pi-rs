use std::process::Stdio;
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncReadExt as _};
use tokio::process::Child;
use tokio::sync::mpsc;

use super::{EffectError, EffectOptions, EffectTimeout, RequestContext, ResourceLease};

const EXIT_STDIO_GRACE: Duration = Duration::from_millis(100);
const TERMINATE_GRACE: Duration = Duration::from_millis(100);

#[derive(Debug, Clone)]
pub struct ProcessRequest {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: Option<String>,
    pub stdin: Option<Vec<u8>>,
    pub options: EffectOptions,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessOutputKind {
    Stdout,
    Stderr,
}

#[derive(Debug)]
pub enum ProcessEvent {
    Output(ProcessOutputKind, Vec<u8>),
    Exit(ProcessOutput),
    Error(EffectError),
}

#[derive(Debug, Clone)]
pub struct ProcessOutput {
    pub code: i64,
    pub killed: bool,
}

#[derive(Debug)]
pub struct ProcessStream {
    rx: mpsc::Receiver<ProcessEvent>,
    pub capacity: usize,
}

impl ProcessStream {
    pub async fn next(&mut self) -> Option<ProcessEvent> {
        self.rx.recv().await
    }
}

struct ProcessGuard {
    child: Child,
    pid: Option<u32>,
}

impl Drop for ProcessGuard {
    fn drop(&mut self) {
        #[cfg(unix)]
        send_signal(self.pid, libc::SIGKILL);
        let _ = self.child.start_kill();
    }
}

#[cfg(unix)]
fn send_signal(pid: Option<u32>, signal: i32) {
    if let Some(pid) = pid.and_then(|pid| i32::try_from(pid).ok()) {
        // SAFETY: signal delivery only; every spawned process starts a new group.
        unsafe {
            if libc::kill(-pid, signal) != 0 {
                libc::kill(pid, signal);
            }
        }
    }
}

#[cfg(not(unix))]
fn send_signal(_pid: Option<u32>, _signal: i32) {}

fn deadline(timeout: EffectTimeout) -> Option<tokio::time::Instant> {
    match timeout {
        EffectTimeout::After(duration) => Some(tokio::time::Instant::now() + duration),
        EffectTimeout::Disabled => None,
    }
}

async fn deadline_wait(deadline: Option<tokio::time::Instant>) {
    match deadline {
        Some(deadline) => tokio::time::sleep_until(deadline).await,
        None => std::future::pending().await,
    }
}

async fn send_event(
    sender: &mpsc::Sender<ProcessEvent>,
    event: ProcessEvent,
    context: &RequestContext,
    deadline: Option<tokio::time::Instant>,
) -> Result<(), EffectError> {
    tokio::select! {
        result = sender.send(event) => result.map_err(|_| EffectError::Cancelled),
        () = context.cancelled() => Err(EffectError::Cancelled),
        () = deadline_wait(deadline) => Err(EffectError::Timeout),
    }
}

async fn terminate(guard: &mut ProcessGuard) -> Option<std::process::ExitStatus> {
    #[cfg(unix)]
    send_signal(guard.pid, libc::SIGTERM);
    #[cfg(not(unix))]
    let _ = guard.child.start_kill();

    match tokio::time::timeout(TERMINATE_GRACE, guard.child.wait()).await {
        Ok(status) => {
            #[cfg(unix)]
            send_signal(guard.pid, libc::SIGKILL);
            status.ok()
        }
        Err(_) => {
            #[cfg(unix)]
            send_signal(guard.pid, libc::SIGKILL);
            let _ = guard.child.start_kill();
            guard.child.wait().await.ok()
        }
    }
}

async fn read_chunk<R: AsyncRead + Unpin>(
    reader: &mut Option<R>,
    buffer: &mut [u8],
) -> std::io::Result<usize> {
    match reader {
        Some(reader) => reader.read(buffer).await,
        None => std::future::pending().await,
    }
}

pub async fn start(
    request: ProcessRequest,
    context: RequestContext,
    lease: Option<ResourceLease>,
) -> Result<ProcessStream, EffectError> {
    let capacity = request.options.stream_capacity;
    let (sender, rx) = mpsc::channel(capacity);
    let mut command = tokio::process::Command::new(&request.program);
    command
        .args(&request.args)
        .stdin(if request.stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    if let Some(cwd) = &request.cwd {
        command.current_dir(cwd);
    }
    #[cfg(unix)]
    // SAFETY: pre_exec calls only async-signal-safe setpgid after fork.
    unsafe {
        command.pre_exec(|| {
            if libc::setpgid(0, 0) == 0 {
                Ok(())
            } else {
                Err(std::io::Error::last_os_error())
            }
        });
    }

    let child = match command.spawn() {
        Ok(child) => child,
        Err(_) => {
            sender
                .send(ProcessEvent::Exit(ProcessOutput {
                    code: 1,
                    killed: false,
                }))
                .await
                .map_err(|_| EffectError::Cancelled)?;
            return Ok(ProcessStream { rx, capacity });
        }
    };
    let pid = child.id();
    let mut guard = ProcessGuard { child, pid };
    if let Some(input) = request.stdin
        && let Some(mut stdin) = guard.child.stdin.take()
    {
        tokio::spawn(async move {
            use tokio::io::AsyncWriteExt as _;
            let _ = stdin.write_all(&input).await;
        });
    }
    let stdout = guard.child.stdout.take();
    let stderr = guard.child.stderr.take();
    let timeout = deadline(request.options.timeout);
    let max_output = request.options.max_output_bytes;

    tokio::spawn(async move {
        let _lease = lease;
        let mut stdout = stdout;
        let mut stderr = stderr;
        let mut stdout_buf = vec![0_u8; 8192];
        let mut stderr_buf = vec![0_u8; 8192];
        let mut total = 0_usize;
        let mut status = None;
        let mut exited_at = None;
        let mut killed = false;
        let mut terminal_error = None;

        loop {
            if stdout.is_none() && stderr.is_none() && status.is_some() {
                break;
            }
            let grace = async {
                match exited_at {
                    Some(at) => tokio::time::sleep_until(at + EXIT_STDIO_GRACE).await,
                    None => std::future::pending().await,
                }
            };
            tokio::select! {
                read = read_chunk(&mut stdout, &mut stdout_buf), if stdout.is_some() => {
                    match read {
                        Ok(0) | Err(_) => stdout = None,
                        Ok(count) => {
                            total = total.saturating_add(count);
                            if total > max_output {
                                terminal_error = Some(EffectError::OutputLimit(max_output));
                                killed = true;
                                break;
                            }
                            if let Err(error) = send_event(
                                &sender,
                                ProcessEvent::Output(ProcessOutputKind::Stdout, stdout_buf[..count].to_vec()),
                                &context,
                                timeout,
                            ).await {
                                terminal_error = Some(error);
                                killed = true;
                                break;
                            }
                        }
                    }
                }
                read = read_chunk(&mut stderr, &mut stderr_buf), if stderr.is_some() => {
                    match read {
                        Ok(0) | Err(_) => stderr = None,
                        Ok(count) => {
                            total = total.saturating_add(count);
                            if total > max_output {
                                terminal_error = Some(EffectError::OutputLimit(max_output));
                                killed = true;
                                break;
                            }
                            if let Err(error) = send_event(
                                &sender,
                                ProcessEvent::Output(ProcessOutputKind::Stderr, stderr_buf[..count].to_vec()),
                                &context,
                                timeout,
                            ).await {
                                terminal_error = Some(error);
                                killed = true;
                                break;
                            }
                        }
                    }
                }
                result = guard.child.wait(), if status.is_none() => {
                    status = result.ok();
                    exited_at = Some(tokio::time::Instant::now());
                }
                () = grace, if exited_at.is_some() => {
                    stdout = None;
                    stderr = None;
                }
                () = context.cancelled() => {
                    terminal_error = Some(EffectError::Cancelled);
                    killed = true;
                    break;
                }
                () = deadline_wait(timeout) => {
                    terminal_error = Some(EffectError::Timeout);
                    killed = true;
                    break;
                }
            }
        }

        if status.is_none() || killed {
            status = terminate(&mut guard).await;
        } else {
            #[cfg(unix)]
            send_signal(guard.pid, libc::SIGKILL);
        }

        if let Some(error) = terminal_error {
            let _ = sender.try_send(ProcessEvent::Error(error));
        } else {
            let code = status.map_or(1, |status| status.code().map_or(0, i64::from));
            let _ = sender
                .send(ProcessEvent::Exit(ProcessOutput { code, killed }))
                .await;
        }
    });

    Ok(ProcessStream { rx, capacity })
}

pub async fn collect(
    request: ProcessRequest,
    context: RequestContext,
) -> Result<(Vec<u8>, Vec<u8>, ProcessOutput), EffectError> {
    let mut stream = start(request, context, None).await?;
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    while let Some(event) = stream.next().await {
        match event {
            ProcessEvent::Output(ProcessOutputKind::Stdout, bytes) => stdout.extend(bytes),
            ProcessEvent::Output(ProcessOutputKind::Stderr, bytes) => stderr.extend(bytes),
            ProcessEvent::Exit(output) => return Ok((stdout, stderr, output)),
            ProcessEvent::Error(error) => return Err(error),
        }
    }
    Err(EffectError::Cancelled)
}

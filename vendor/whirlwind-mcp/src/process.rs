//! Bounded host-process runner. No shell, credential arguments, or global profile writes.
use anyhow::{Result, bail};
use std::{
    io::Read,
    path::Path,
    process::{Command, Stdio},
    sync::{Arc, Mutex, mpsc},
    time::{Duration, Instant},
};
const LIMIT: usize = 1024 * 1024;
struct Capture {
    bytes: Arc<Mutex<Vec<u8>>>,
    done: mpsc::Receiver<()>,
}
fn capture(mut input: impl Read + Send + 'static) -> Capture {
    let bytes = Arc::new(Mutex::new(Vec::new()));
    let collected = bytes.clone();
    let (tx, done) = mpsc::channel();
    std::thread::spawn(move || {
        let mut chunk = [0; 4096];
        loop {
            match input.read(&mut chunk) {
                Ok(0) => break,
                Ok(n) => {
                    let mut out = collected.lock().unwrap_or_else(|p| p.into_inner());
                    let keep = n.min((LIMIT + 1).saturating_sub(out.len()));
                    out.extend_from_slice(&chunk[..keep]);
                }
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => break,
            }
        }
        let _ = tx.send(());
    });
    Capture { bytes, done }
}
pub(crate) fn run(
    program: &Path,
    args: &[String],
    codex: &Path,
    claude: Option<&Path>,
    timeout: Duration,
) -> Result<String> {
    let mut command = Command::new(program);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("CODEX_HOME", codex);
    if let Some(claude) = claude {
        command.env("CLAUDE_CONFIG_DIR", claude);
    } else {
        command.env_remove("CLAUDE_CONFIG_DIR");
    }
    let mut command = process_wrap::std::CommandWrap::from(command);
    #[cfg(unix)]
    command.wrap(process_wrap::std::ProcessGroup::leader());
    #[cfg(windows)]
    {
        let mut flags = process_wrap::std::CreationFlags(Default::default());
        flags.0.0 = 0x0800_0000;
        command.wrap(flags).wrap(process_wrap::std::JobObject);
    }
    let mut child = command.spawn()?;
    let out = child.stdout().take().map(capture);
    let err = child.stderr().take().map(capture);
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(50)),
            result => {
                let _ = child.kill();
                let _ = child.wait();
                if let Err(error) = result {
                    return Err(error.into());
                }
                bail!("Assistant command timed out. Retry from AI assistants.");
            }
        }
    };
    let drain_deadline = Instant::now() + Duration::from_secs(1);
    let read = |capture: Option<Capture>| -> Vec<u8> {
        capture
            .map(|c| {
                let _ = c
                    .done
                    .recv_timeout(drain_deadline.saturating_duration_since(Instant::now()));
                c.bytes.lock().unwrap_or_else(|p| p.into_inner()).clone()
            })
            .unwrap_or_default()
    };
    let bytes = read(out);
    let errors = read(err);
    if !status.success() {
        bail!(
            "Assistant command failed (exit {}). Check the assistant's plugin settings, then use Repair.",
            status
                .code()
                .map(|n| n.to_string())
                .unwrap_or_else(|| "signal".into())
        );
    }
    if bytes.len() > LIMIT || errors.len() > LIMIT {
        bail!("Assistant command output exceeded the limit; nothing further was changed.");
    }
    Ok(String::from_utf8(bytes)?)
}
#[cfg(all(test, unix))]
mod tests {
    use super::*;
    #[test]
    fn output_and_lifetime_are_bounded() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            run(
                Path::new("/bin/echo"),
                &["native".into()],
                dir.path(),
                Some(dir.path()),
                Duration::from_secs(2)
            )
            .unwrap(),
            "native\n"
        );
        let start = Instant::now();
        assert!(
            run(
                Path::new("/bin/sleep"),
                &["30".into()],
                dir.path(),
                Some(dir.path()),
                Duration::from_millis(100)
            )
            .is_err()
        );
        assert!(start.elapsed() < Duration::from_secs(3));
    }
}

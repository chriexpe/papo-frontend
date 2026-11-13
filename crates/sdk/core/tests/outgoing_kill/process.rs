//! Owns subprocess pipes, bounded readiness waits and guaranteed SIGKILL cleanup; sends remain in the child.

use super::Result;
use std::{
    io::{BufRead, BufReader},
    os::unix::process::ExitStatusExt,
    process::{Child, Command, Stdio},
    sync::mpsc,
    thread::JoinHandle,
    time::Duration,
};

pub(super) struct Process {
    child: Child,
    lines: mpsc::Receiver<String>,
    reader: Option<JoinHandle<()>>,
}
impl Process {
    pub fn start(mut command: Command) -> Result<Self> {
        let mut child = command
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let output = child.stdout.take().ok_or("missing child stdout")?;
        let (tx, lines) = mpsc::channel();
        let reader = std::thread::spawn(move || {
            for line in BufReader::new(output)
                .lines()
                .map_while(std::result::Result::ok)
            {
                let _ = tx.send(line);
            }
        });
        Ok(Self {
            child,
            lines,
            reader: Some(reader),
        })
    }
    pub fn ready(&self, needs_probe: bool) -> Result {
        let mut acknowledged = false;
        let mut boundary = !needs_probe;
        while !acknowledged || !boundary {
            let line = self
                .lines
                .recv_timeout(Duration::from_secs(30))
                .map_err(|_| "child did not reach controlled boundary")?;
            acknowledged |= line.contains("PAPO_ENQUEUED");
            boundary |= line.contains("PAPO_BOUNDARY");
        }
        Ok(())
    }
    pub fn kill(&mut self) -> Result {
        self.child.kill()?;
        assert_eq!(self.child.wait()?.signal(), Some(9));
        self.join()
    }
    fn join(&mut self) -> Result {
        if let Some(reader) = self.reader.take() {
            reader.join().map_err(|_| "child output reader failed")?;
        }
        Ok(())
    }
}
impl Drop for Process {
    fn drop(&mut self) {
        if !matches!(self.child.try_wait(), Ok(Some(_))) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
        let _ = self.join();
    }
}

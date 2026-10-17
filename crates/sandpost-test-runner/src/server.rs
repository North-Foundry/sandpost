use crate::{Result, http::HttpClient};
use std::{
    fs::{self, OpenOptions},
    net::TcpListener,
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus},
    thread,
    time::{Duration, Instant},
};

pub const SECRET_SENTINEL: &str = "sandpost-smoke-secret-sentinel";

pub struct ServerConfiguration {
    pub binary: PathBuf,
    pub directory: PathBuf,
    pub web_port: u16,
    pub mail_port: u16,
    pub imap_port: u16,
}

pub struct ServerProcess {
    child: Child,
    directory: PathBuf,
    stopped: bool,
}

impl ServerConfiguration {
    /// Reserve distinct ephemeral ports until all three listener addresses are selected.
    pub fn new(binary: &Path, directory: &Path) -> Result<Self> {
        let listeners = [
            TcpListener::bind(("127.0.0.1", 0))?,
            TcpListener::bind(("127.0.0.1", 0))?,
            TcpListener::bind(("127.0.0.1", 0))?,
        ];
        fs::write(directory.join("config.toml"), "")?;
        Ok(Self {
            binary: binary.to_owned(),
            directory: directory.to_owned(),
            web_port: listeners[0].local_addr()?.port(),
            mail_port: listeners[1].local_addr()?.port(),
            imap_port: listeners[2].local_addr()?.port(),
        })
    }

    /// Build a subprocess environment isolated from the user's Sandpost installation.
    pub fn command(&self) -> Command {
        let mut command = Command::new(&self.binary);
        for (name, _) in std::env::vars_os() {
            if name.to_string_lossy().starts_with("SANDPOST_") {
                command.env_remove(name);
            }
        }
        command
            .env("SANDPOST_CONFIG", self.directory.join("config.toml"))
            .env("SANDPOST_DATA_DIRECTORY", &self.directory)
            .env("SANDPOST_DATABASE_PATH", self.database_path())
            .env("SANDPOST_HTTP_BIND_ADDRESS", "127.0.0.1")
            .env("SANDPOST_HTTP_PORT", self.web_port.to_string())
            .env("SANDPOST_SMTP_BIND_ADDRESS", "127.0.0.1")
            .env("SANDPOST_SMTP_PORT", self.mail_port.to_string())
            .env("SANDPOST_IMAP_ENABLED", "true")
            .env("SANDPOST_IMAP_BIND_ADDRESS", "127.0.0.1")
            .env("SANDPOST_IMAP_PORT", self.imap_port.to_string())
            .env("SANDPOST_LOG_LEVEL", "info")
            .env("SMTP_PASSWORD", SECRET_SENTINEL);
        command
    }

    /// Return the isolated SQLite file used by the child server.
    pub fn database_path(&self) -> PathBuf {
        self.directory.join("sandpost.sqlite3")
    }

    /// Start the child with append-only logs, killing and reaping it on any later failure.
    pub fn spawn(&self, command: &mut Command) -> Result<ServerProcess> {
        let output = OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.directory.join("server.log"))?;
        let errors = OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.directory.join("server.err"))?;
        let child = command.stdout(output).stderr(errors).spawn()?;
        Ok(ServerProcess {
            child,
            directory: self.directory.clone(),
            stopped: false,
        })
    }

    /// Wait for a healthy HTTP listener while detecting startup failures and timeouts.
    pub fn start(&self, client: &HttpClient) -> Result<ServerProcess> {
        let mut process = self.spawn(&mut self.command())?;
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            process.require_running()?;
            if let Ok(response) = client.get_json("/api/v1/health")
                && response["status"] == "ok"
            {
                return Ok(process);
            }
            if Instant::now() >= deadline {
                return Err(format!("server readiness timeout: {}", self.logs()?).into());
            }
            thread::sleep(Duration::from_millis(50));
        }
    }

    /// Read stdout and stderr together for failure diagnostics and secret checks.
    pub fn logs(&self) -> Result<String> {
        Ok(format!("{}{}", self.stdout()?, self.stderr()?))
    }

    /// Read the plain-text startup summaries accumulated across restarts.
    pub fn stdout(&self) -> Result<String> {
        Ok(fs::read_to_string(self.directory.join("server.log"))?)
    }

    /// Read tracing output accumulated across restarts.
    pub fn stderr(&self) -> Result<String> {
        Ok(fs::read_to_string(self.directory.join("server.err"))?)
    }

    /// Run a maintenance or failed-startup command with bounded execution and captured logs.
    pub fn complete(
        &self,
        command: &mut Command,
        timeout: Duration,
    ) -> Result<(ExitStatus, String, String)> {
        let output = tempfile::tempfile()?;
        let errors = tempfile::tempfile()?;
        let child = command
            .stdout(output.try_clone()?)
            .stderr(errors.try_clone()?)
            .spawn()?;
        let mut process = ServerProcess {
            child,
            directory: self.directory.clone(),
            stopped: false,
        };
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(status) = process.child.try_wait()? {
                process.stopped = true;
                use std::io::{Read, Seek, SeekFrom};
                let mut output = output;
                let mut errors = errors;
                output.seek(SeekFrom::Start(0))?;
                errors.seek(SeekFrom::Start(0))?;
                let mut output_text = String::new();
                let mut error_text = String::new();
                output.read_to_string(&mut output_text)?;
                errors.read_to_string(&mut error_text)?;
                return Ok((status, output_text, error_text));
            }
            if Instant::now() >= deadline {
                return Err("server command timed out".into());
            }
            thread::sleep(Duration::from_millis(25));
        }
    }
}

impl ServerProcess {
    /// Fail immediately when a child exits before the requested shutdown.
    pub fn require_running(&mut self) -> Result<()> {
        if let Some(status) = self.child.try_wait()? {
            return Err(format!(
                "server exited early ({status}): {}{}",
                fs::read_to_string(self.directory.join("server.log"))?,
                fs::read_to_string(self.directory.join("server.err"))?
            )
            .into());
        }
        Ok(())
    }

    /// Require graceful SIGTERM shutdown on Unix, or forced termination on Windows, within nine seconds.
    pub fn stop(mut self) -> Result<()> {
        self.require_running()?;
        #[cfg(unix)]
        {
            // SAFETY: the live, unreaped child owns this process identifier.
            if unsafe { libc::kill(self.child.id() as libc::pid_t, libc::SIGTERM) } != 0 {
                return Err(std::io::Error::last_os_error().into());
            }
        }
        #[cfg(not(unix))]
        self.child.kill()?;
        let deadline = Instant::now() + Duration::from_secs(9);
        loop {
            if let Some(status) = self.child.try_wait()? {
                self.stopped = true;
                #[cfg(unix)]
                assert!(status.success(), "server shutdown failed: {status}");
                #[cfg(windows)]
                assert_eq!(
                    status.code(),
                    Some(1),
                    "unexpected forced shutdown: {status}"
                );
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err("server did not stop within nine seconds".into());
            }
            thread::sleep(Duration::from_millis(25));
        }
    }
}

impl Drop for ServerProcess {
    /// Always kill and reap unfinished children, including during assertion unwinding.
    fn drop(&mut self) {
        if !self.stopped {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

//! A private D-Bus bus for headless tests: its own `dbus-daemon`, in a
//! temporary directory, gone when the test ends.
//!
//! Owns only the daemon's lifetime and its address. The bus starts no
//! services (its configuration names no service directories), so a test that
//! calls a name nobody owns gets an error instead of the real portal or
//! logind. What the test serves on it is the test's.

use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};

/// A running private bus. Dropping it stops the daemon.
pub(crate) struct PrivateBus {
    daemon: Child,
    address: String,
    _dir: tempfile::TempDir,
}

/// Everyone may own any name and talk to anyone; no service is activated.
const CONFIG: &str = r#"<!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN"
 "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
<busconfig>
  <type>session</type>
  <listen>unix:dir=DIR</listen>
  <policy context="default">
    <allow send_destination="*" eavesdrop="true"/>
    <allow eavesdrop="true"/>
    <allow own="*"/>
  </policy>
</busconfig>
"#;

impl PrivateBus {
    /// Starts the daemon and waits for its address.
    ///
    /// # Panics
    ///
    /// When `dbus-daemon` is not installed or does not start: these tests
    /// need it (`scripts/deps.sh`).
    pub(crate) fn start() -> PrivateBus {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let config = dir.path().join("bus.conf");
        let listen = dir.path().to_str().expect("a UTF-8 temporary path");
        std::fs::write(&config, CONFIG.replace("DIR", listen)).expect("the bus configuration");
        let mut daemon = Command::new("dbus-daemon")
            .arg("--nofork")
            .arg("--print-address")
            .arg(format!("--config-file={}", config.display()))
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("dbus-daemon, which these tests need, to start");
        let stdout = daemon.stdout.take().expect("the daemon's stdout");
        let mut address = String::new();
        BufReader::new(stdout)
            .read_line(&mut address)
            .expect("the daemon's address");
        PrivateBus {
            daemon,
            address: address.trim().to_owned(),
            _dir: dir,
        }
    }

    /// A new connection to the bus.
    pub(crate) async fn connect(&self) -> zbus::Connection {
        self.builder()
            .build()
            .await
            .expect("a connection to the private bus")
    }

    /// A connection builder for the bus, for a test that serves on it.
    pub(crate) fn builder(&self) -> zbus::connection::Builder<'static> {
        zbus::connection::Builder::address(self.address.as_str())
    }
}

impl Drop for PrivateBus {
    fn drop(&mut self) {
        let _ = self.daemon.kill();
        let _ = self.daemon.wait();
    }
}

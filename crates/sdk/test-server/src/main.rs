//! Owns CLI configuration and process shutdown; leaves backend behavior and networking to the library.

use std::{env, path::PathBuf, process::ExitCode};

use papo_test_server::{Config, Error, TestServer};

fn options() -> Result<(Config, u16), Error> {
    let mut arguments = env::args_os().skip(1);
    let mut port = 0;
    let mut path = None;
    while let Some(argument) = arguments.next() {
        match argument.to_str() {
            Some("--port") => {
                port = arguments
                    .next()
                    .and_then(|value| value.to_str().and_then(|value| value.parse().ok()))
                    .ok_or(Error::Arguments)?;
            }
            Some("--config") => {
                path = Some(PathBuf::from(arguments.next().ok_or(Error::Arguments)?));
            }
            _ => return Err(Error::Arguments),
        }
    }
    // Configuration I/O completes before creating any Tokio worker.
    let config = path.map_or_else(
        || Ok(Config::default()),
        |path| {
            let bytes = std::fs::read(path).map_err(|_| Error::ConfigurationRead)?;
            serde_json::from_slice(&bytes).map_err(|_| Error::ConfigurationRead)
        },
    )?;
    Ok((config, port))
}

fn run() -> Result<(), Error> {
    let (config, port) = options()?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .map_err(|_| Error::Runtime)?;
    runtime.block_on(async move {
        let server = TestServer::on_port(config, port).await?;
        println!("papo-test-server ready");
        tokio::signal::ctrl_c().await.map_err(|_| Error::Signal)?;
        server.shutdown().await
    })
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

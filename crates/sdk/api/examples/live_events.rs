//! Signs in and prints only static event kinds and validated UUIDs; never logs content or credentials.

use papo_api::{RestClient, gateway::Gateway, wire::LoginRequest};
use std::io::{self, BufRead};

fn input(name: &str) -> Result<String, &'static str> {
    if let Ok(value) = std::env::var(name) {
        return Ok(value);
    }
    let mut line = String::new();
    io::stdin()
        .lock()
        .read_line(&mut line)
        .map_err(|_| "input failed")?;
    if line.is_empty() {
        return Err("input missing");
    }
    Ok(line.trim_end_matches(['\r', '\n']).into())
}

#[tokio::main]
async fn main() {
    if let Err(stage) = run().await {
        eprintln!("live event example failed: {stage}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), &'static str> {
    // Stdin fallback consumes address, username, password in that order, without printing them.
    let address = input("PAPO_ADDRESS")?;
    let username = input("PAPO_USERNAME")?;
    let password = input("PAPO_PASSWORD")?;
    let client = if std::env::var_os("PAPO_LOOPBACK_HTTP").is_some() {
        RestClient::loopback(&address)
    } else {
        RestClient::new(&address)
    }
    .map_err(|_| "client construction")?;
    if let Ok(server_password) = std::env::var("PAPO_SERVER_PASSWORD") {
        client
            .login_server(&papo_api::wire::LoginServerRequest { server_password })
            .await
            .map_err(|_| "server login")?;
    }
    client
        .login(&LoginRequest { username, password })
        .await
        .map_err(|_| "account login")?;
    let mut gateway = Gateway::connect(&client)
        .await
        .map_err(|_| "gateway connection")?;
    // Optional event count makes local smoke verification bounded without adding reconnect policy.
    let limit = std::env::var("PAPO_EVENT_LIMIT")
        .ok()
        .and_then(|value| value.parse::<usize>().ok());
    let mut count = 0;
    while let Some(event) = gateway.next().await.map_err(|_| "event read")? {
        print!("{}", event.kind());
        for id in event.ids() {
            print!(" {id}");
        }
        println!();
        count += 1;
        if limit.is_some_and(|limit| count >= limit) {
            break;
        }
    }
    gateway.close().await.map_err(|_| "gateway close")
}

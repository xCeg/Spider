//! The debug-port (CDP) transport against a simulated Discord renderer.
//!
//! `tests/fake_discord.mjs` (Node, no dependency) plays Chromium's debug port and runs the
//! very script Spider sends to the real Discord, in a page that imitates Discord's module cache.
//! This checks everything on Spider's side: the page lookup (client, overlay, service worker),
//! the websocket, the Flux dispatcher search, the activity it builds, the wait for a slow store
//! and the clear. Skipped when Node is not installed.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use serde_json::{Value, json};
use spider::engine::{self, Client, Config, Mode, Transport};

struct FakeDiscord {
    child: Child,
    port: u16,
}

impl Drop for FakeDiscord {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

fn fake_discord() -> Option<FakeDiscord> {
    let script = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fake_discord.mjs");
    let mut child = Command::new("node")
        .arg(script)
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .ok()?;
    let mut line = String::new();
    BufReader::new(child.stdout.take()?).read_line(&mut line).ok()?;
    let port = line.trim().parse().ok()?;
    Some(FakeDiscord { child, port })
}

/// The activities a page's store holds, by socket id.
fn store(port: u16, page: &str) -> Value {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("the fake Discord answers");
    write!(stream, "GET /state/{page} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n")
        .unwrap();
    let mut text = String::new();
    stream.read_to_string(&mut text).unwrap();
    serde_json::from_str(text.split_once("\r\n\r\n").unwrap().1).unwrap()
}

fn streaming(slot: u8) -> Config {
    Config {
        app_id: 123_456_789_012_345_678,
        slot,
        name: "Spider".into(),
        details: "En direct".into(),
        state: "Test".into(),
        kind: 1,
        platform: Some("xbox"),
        stream_url: "https://www.twitch.tv/spider".into(),
        buttons: vec![("Site".into(), "https://example.test/".into())],
        timer: true,
        ..Config::default()
    }
}

#[test]
fn cards_reach_the_chosen_discord_through_the_debug_port() {
    let Some(fake) = fake_discord() else {
        eprintln!("Node introuvable : test du port de debug sauté");
        return;
    };
    let port = fake.port;
    engine::set_debug_port(port);

    // Discord (stable): its service worker and its overlay come first in the list and must be
    // left alone; the cards land in the main window's store, one socket per slot.
    engine::set_client(Client::Stable);
    assert!(engine::available());
    assert_eq!(engine::debug_port_client(), Some(Client::Stable));
    let mut listening = streaming(2);
    listening.kind = 2;
    listening.platform = None;
    listening.name = "Musique".into();
    let cards = vec![streaming(1), listening];
    assert_eq!(engine::publish(&cards, Mode::Cdp, None, false), Ok(Transport::Cdp));
    let stable = store(port, "stable");
    let first = &stable["spider1"];
    assert_eq!(first["type"], 1, "{stable}");
    assert_eq!(first["name"], "Spider");
    assert_eq!(first["details"], "En direct");
    assert_eq!(first["url"], "https://www.twitch.tv/spider");
    assert_eq!(first["platform"], "xbox");
    assert_eq!(first["buttons"], json!(["Site"]));
    assert_eq!(first["metadata"]["button_urls"], json!(["https://example.test/"]));
    assert_eq!(first["flags"], 1);
    assert_eq!(first["instance"], true);
    assert!(first["timestamps"]["start"].is_u64());
    let second = &stable["spider2"];
    assert_eq!(second["type"], 2);
    assert!(second["timestamps"]["end"].as_u64() > second["timestamps"]["start"].as_u64());
    for untouched in ["worker", "overlay", "ptb", "canary"] {
        assert_eq!(store(port, untouched), json!({}), "{untouched} must stay empty");
    }

    // A refresh sends the same cards again through the kept dispatcher: nothing piles up.
    assert_eq!(engine::publish(&cards, Mode::Cdp, Some(Transport::Cdp), false), Ok(Transport::Cdp));
    assert_eq!(store(port, "stable").as_object().map(|cards| cards.len()), Some(2));

    // A change of the cards is a hand-over: the old ones go, the new one stays alone.
    let mut playing = streaming(1);
    playing.kind = 0;
    assert_eq!(engine::publish(&[playing], Mode::Cdp, Some(Transport::Cdp), true), Ok(Transport::Cdp));
    let stable = store(port, "stable");
    assert_eq!(stable.as_object().map(|cards| cards.len()), Some(1), "{stable}");
    assert_eq!(stable["spider1"]["type"], 0);

    // Clearing empties the store.
    engine::clear(Mode::Cdp).unwrap();
    assert_eq!(store(port, "stable"), json!({}));

    // Discord PTB is another account: the cards go to its own window only.
    engine::set_client(Client::Ptb);
    assert_eq!(engine::publish(&cards[..1], Mode::Cdp, None, false), Ok(Transport::Cdp));
    assert!(store(port, "ptb")["spider1"].is_object());
    assert_eq!(store(port, "stable"), json!({}));
    engine::clear(Mode::Cdp).unwrap();
    assert_eq!(store(port, "ptb"), json!({}));

    // Discord Canary's store shows the card a moment after the dispatch: the push waits for
    // it instead of calling that a failure (which used to switch to the pipe).
    engine::set_client(Client::Canary);
    assert_eq!(engine::publish(&cards[..1], Mode::Cdp, None, false), Ok(Transport::Cdp));
    std::thread::sleep(Duration::from_millis(100));
    assert!(store(port, "canary")["spider1"].is_object());
    engine::clear(Mode::Cdp).unwrap();
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(store(port, "canary"), json!({}));
}

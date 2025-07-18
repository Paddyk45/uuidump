#![feature(iter_array_chunks)]
#![warn(clippy::nursery, clippy::pedantic)]

use bpaf::Bpaf;
use lazy_static::lazy_static;
use serde_json::json;
use std::collections::HashSet;
use std::io::{Write, stdout};
use std::str::FromStr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};
use std::thread::{sleep, spawn};
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use uuid::Uuid;

#[derive(Debug, Clone, Bpaf)]
#[bpaf(options)]
struct Cli {
    #[bpaf(
        argument("WORDLIST"),
        short('w'),
        long("wordlist-path"),
        help("[path] the file to pull the names from. all non-mc-name characters will be nuked.")
    )]
    wordlist_path: String,
    #[bpaf(
        argument("THREADS"),
        short('t'),
        fallback(80),
        help("[num] how many threads to spawn for making requests.")
    )]
    threads: usize,
    #[bpaf(
        argument("OUTPUT"),
        short('o'),
        help("[path] where to output uuids to.")
    )]
    output_path: String,
    #[bpaf(
        argument("IGNORED"),
        short('i'),
        fallback(None),
        help(
            "[path] which uuids to ignore if found. useful in combination with one of mats uuid dumps. if not given, don't ignore any uuids."
        )
    )]
    ignored: Option<String>,
    #[bpaf(
        argument("INGNORED_TRUNCATION"),
        short('r'),
        fallback(None),
        help(
            "[num] amount of hex digits to keep from from the uuids (8 for laby). no truncation if not given."
        )
    )]
    ignored_truncation: Option<usize>,
    #[bpaf(
        argument("SUFFIXES"),
        short('s'),
        fallback(None),
        help(
            "[path] list of suffixes to append to each word in the wordlist. words with no suffixes will not be kept. no suffixing if not given."
        )
    )]
    suffixes: Option<String>,
    #[bpaf(
        short('a'),
        fallback(false),
        switch,
        help("whether to print ignored uuids in a gray color.")
    )]
    print_ignored: bool,
}

const ALLOWED_CHARS: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz1234567890_";
const MOWOJANG: &str = "https://mowojang.matdoes.dev";

lazy_static! {
    static ref CLIENT: reqwest::Client = reqwest::Client::new();
    static ref UUID_COUNTER: Arc<AtomicUsize> = Arc::new(AtomicUsize::new(0));
    static ref UUID_ALL_COUNTER: Arc<AtomicUsize> = Arc::new(AtomicUsize::new(0));
    static ref REQ_COUNTER: Arc<AtomicUsize> = Arc::new(AtomicUsize::new(0));
    static ref TOTAL_REQUESTS: OnceLock<usize> = OnceLock::new();
}

#[tokio::main]
async fn main() -> eyre::Result<()> {
    let args: Cli = cli().run();
    if tokio::fs::try_exists(&args.output_path).await? {
        eprintln!("warn: output file already exists, found uuids will be appended.");
    }
    eprintln!("parsing wordlist");
    let wordlist_f = tokio::fs::read_to_string(args.wordlist_path).await?;
    let mut wordlist = wordlist_f
        .lines()
        .map(|w| {
            w.chars()
                .filter(|c| ALLOWED_CHARS.contains(*c))
                .collect::<String>()
        })
        .filter(|w| (3..16).contains(&w.len()))
        .map(|w| w.to_ascii_lowercase())
        .collect::<Vec<String>>();
    wordlist.sort();
    wordlist.dedup();
    drop(wordlist_f);

    let suffixes = if let Some(suffixes) = args.suffixes {
        let suffixes = tokio::fs::read_to_string(suffixes).await?;
        suffixes.lines().map(String::from).collect::<Vec<String>>()
    } else {
        vec![String::new()]
    };

    eprintln!("loaded {} names", wordlist.len());

    eprintln!("parsing ignored uuids");
    let ignored = if let Some(ignored) = args.ignored {
        let ignored_f = tokio::fs::read_to_string(ignored).await?;
        let ignored: HashSet<Uuid> = ignored_f
            .lines()
            .map(String::from)
            .map(|mut u| {
                if args.ignored_truncation.is_some() {
                    u = format!("{u}{}", "0".repeat(32 - u.len()));
                }
                Uuid::from_str(&u).expect("failed to parse uuid")
            })
            .collect::<HashSet<_>>();
        ignored
    } else {
        HashSet::default()
    };

    eprintln!("{} uuids ignored", ignored.len());

    let (tx, rx) = unbounded_channel::<(Uuid, String)>();
    tokio::spawn(handler(
        rx,
        ignored,
        args.ignored_truncation,
        args.output_path,
        args.print_ignored,
    ));

    let words = wordlist.len();
    let wordlist_parts = wordlist.chunks(words / args.threads.clamp(1, words));

    eprintln!("spawning tasks");
    let mut handles = vec![];
    for w in wordlist_parts {
        let suffixes = suffixes.clone();
        handles.push(tokio::spawn(request_thread(
            tx.clone(),
            w.to_vec(),
            suffixes,
        )));
    }

    spawn(display_thread);

    for h in handles {
        h.await?;
    }

    Ok(())
}

// thread which scrapes uuids and sends found uuids to the handler
async fn request_thread(
    tx: UnboundedSender<(Uuid, String)>,
    wordlist_part: Vec<String>,
    suffixes: Vec<String>,
) {
    for wordlist_chunk in wordlist_part.chunks(100) {
        let mut wordlist_suffixed = vec![];
        for word in wordlist_chunk {
            for suf in &suffixes {
                wordlist_suffixed.push(format!("{word}{suf}"));
            }
        }

        for w in wordlist_suffixed.chunks(10) {
            let uuids = request(w.to_vec()).await;
            for uuid_name in uuids {
                tx.send(uuid_name).unwrap();
            }
        }
    }
}

// thread which handles ignoring uuids and outputting uuids to the file
async fn handler(
    mut rx: UnboundedReceiver<(Uuid, String)>,
    ignored: HashSet<Uuid>,
    ignored_truncation: Option<usize>,
    out: String,
    print_ignored: bool,
) {
    let mut output_f = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(out)
        .await
        .expect("failed to open output file");

    while let Some((uuid, name)) = rx.recv().await {
        if ignored.contains(&uuid)
            || (ignored_truncation.is_some_and(|trunc| {
                ignored.contains(&Uuid::from_u128(
                    uuid.as_u128() & (u128::MAX << (128 - (trunc * 4) as u128)),
                ))
            }))
        {
            if print_ignored {
                println!("\x1b[2K\r\x1b[38;5;241m{uuid}:{name}\x1b[0m");
                print_status();
            }
            continue;
        }

        UUID_COUNTER.fetch_add(1, Ordering::SeqCst);

        eprintln!("\x1b[2K\r{uuid}:{name}");
        print_status();

        output_f
            .write_all(format!("{uuid}\n").as_bytes())
            .await
            .expect("failed to write to file");
    }
}

async fn request(names: Vec<String>) -> Vec<(Uuid, String)> {
    assert!(names.len() <= 10, "too many uuids :(");

    let res: serde_json::Value = match CLIENT
        .post(MOWOJANG)
        .header("content-type", "application/json")
        .body(json!(names).to_string())
        .send()
        .await
    {
        Ok(res) => res.json().await.unwrap(),
        Err(e) => {
            eprintln!("mowojang api request failed: {e:?}");
            return vec![];
        }
    };
    REQ_COUNTER.fetch_add(1, Ordering::SeqCst);
    let mut pls = vec![];
    for pl in res.as_array().unwrap() {
        UUID_ALL_COUNTER.fetch_add(1, Ordering::SeqCst);
        pls.push((Uuid::from_str(pl["id"].as_str().unwrap()).unwrap(), pl["name"].as_str().unwrap().to_string()));
    }
    pls
}

fn display_thread() {
    loop {
        print_status();
        sleep(Duration::from_secs(1));
    }
}

fn print_status() {
    print!(
        "\x1b[2K\rreqs: {} | found: {} ({} total)",
        REQ_COUNTER.load(Ordering::SeqCst),
        UUID_COUNTER.load(Ordering::SeqCst),
        UUID_ALL_COUNTER.load(Ordering::SeqCst)
    );
    let _ = stdout().lock().flush();
}

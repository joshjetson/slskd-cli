mod cli;
mod client;
mod config;
mod format;
mod models;
mod rank;
mod tui;

use anyhow::{Context, Result};
use clap::Parser;
use cli::{BrowseArgs, Cli, Command, GetArgs, SearchArgs, TransferArgs};
use client::Client;
use config::Config;
use format as fmt;
use models::flatten;
use std::io::Write;
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<()> {
    let args = Cli::parse();

    // `init` is the one command that runs without a working config.
    if matches!(args.command, Some(Command::Init)) {
        let path = Config::write_default()?;
        println!("wrote {}", path.display());
        println!("edit it to point at your slskd instance, then put your API key in");
        println!("~/.config/slskd-cli/.apikey (chmod 600)");
        return Ok(());
    }

    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(120))
        .build()?;

    let cfg = Config::load()?;
    let key = cfg.key()?;
    let chosen = cfg
        .select(&http, args.url.as_deref(), args.endpoint.as_deref())
        .await?;
    if args.verbose {
        eprintln!("endpoint: {} ({})", chosen.url, chosen.name);
    }
    let base = chosen.url;
    let api = Client::new(http, base, key);

    match args.command {
        None => tui::run(api).await,
        Some(Command::Init) => unreachable!("handled above"),
        Some(Command::Status) => status(&api).await,
        Some(Command::Search(a)) => search(&api, &a).await,
        Some(Command::Get(a)) => get(&api, &a).await,
        Some(Command::Transfers(a)) => transfers(&api, &a).await,
        Some(Command::Browse(a)) => browse(&api, &a).await,
    }
}

async fn status(api: &Client) -> Result<()> {
    let app = api.application().await?;
    println!("endpoint   {}", api.base());
    println!("slskd      {}", app.version.current);
    if app.version.is_update_available {
        println!("           update available: {}", app.version.latest);
    }
    println!("server     {} ({})", app.server.state, app.server.address);
    println!("user       {}", app.user.username);
    println!(
        "shares     {} files, {} directories (ready: {})",
        app.shares.files, app.shares.directories, app.shares.ready
    );
    if !app.server.is_logged_in {
        println!();
        println!("slskd is not logged in, so searches will fail. It reconnects on its");
        println!("own with exponential backoff; searching too fast is a common cause.");
    }
    if app.shares.files == 0 {
        println!();
        println!("note: you are sharing nothing. Soulseek users routinely ban");
        println!("      zero-share accounts, which will quietly cut off downloads.");
    }
    Ok(())
}

async fn run_search(api: &Client, a: &SearchArgs) -> Result<Vec<models::Candidate>> {
    let query = a.query_string();
    if query.trim().is_empty() {
        anyhow::bail!("nothing to search for");
    }
    eprint!("searching {query:?} ");
    let _ = std::io::stderr().flush();

    let res = api
        .search(&query, Duration::from_secs(a.wait))
        .await
        .context("search failed")?;

    eprintln!(
        "— {} responses, {} files ({} locked)",
        res.response_count, res.file_count, res.locked_file_count
    );

    Ok(rank::rank(&res.responses, &a.to_filter()))
}

fn print_candidates(cands: &[models::Candidate], limit: usize) {
    if cands.is_empty() {
        println!("no matching files");
        return;
    }
    println!(
        "{:<3} {:<18} {:<4} {:>4} {:>8} {:>7} {:>6} {:>5}  {:<38}  {}",
        "#", "user", "slot", "q", "speed", "size", "rate", "len", "file", "folder"
    );
    for (i, c) in cands.iter().take(limit).enumerate() {
        println!(
            "{:<3} {:<18} {:<4} {:>4} {:>8} {:>7} {:>6} {:>5}  {:<38}  {}",
            i + 1,
            fmt::truncate(&c.username, 18),
            if c.has_free_slot { "yes" } else { "no" },
            c.queue_length,
            fmt::speed(c.upload_speed as f64),
            fmt::bytes(c.file.size),
            fmt::bitrate(c.file.bit_rate),
            fmt::duration(c.file.length),
            fmt::truncate(c.name(), 38),
            fmt::truncate(c.album(), 30),
        );
    }
    if cands.len() > limit {
        println!("... and {} more", cands.len() - limit);
    }
}

async fn search(api: &Client, a: &SearchArgs) -> Result<()> {
    let cands = run_search(api, a).await?;
    print_candidates(&cands, a.limit);
    Ok(())
}

async fn get(api: &Client, a: &GetArgs) -> Result<()> {
    let cands = run_search(api, &a.search).await?;
    if cands.is_empty() {
        anyhow::bail!("nothing matched — try --any-format or --no-duration-check");
    }
    let take = a.count.max(1).min(cands.len());
    let chosen = &cands[..take];

    println!("picked:");
    print_candidates(chosen, take);

    if !a.yes {
        print!("\nqueue {} file(s)? [y/N] ", take);
        std::io::stdout().flush()?;
        let mut line = String::new();
        std::io::stdin().read_line(&mut line)?;
        if !matches!(line.trim().to_lowercase().as_str(), "y" | "yes") {
            println!("cancelled");
            return Ok(());
        }
    }

    for c in chosen {
        match api.enqueue(&c.username, std::slice::from_ref(&c.file)).await {
            Ok(()) => println!("queued  {}  from {}", c.name(), c.username),
            Err(e) => eprintln!("failed  {}  from {}: {e}", c.name(), c.username),
        }
    }
    Ok(())
}

async fn transfers(api: &Client, a: &TransferArgs) -> Result<()> {
    if a.clear {
        api.clear_completed().await?;
        println!("cleared completed and errored records");
        return Ok(());
    }

    loop {
        let users = if a.uploads {
            api.uploads().await?
        } else {
            api.downloads().await?
        };
        let list = flatten(&users);

        if list.is_empty() {
            println!("no transfers");
            return Ok(());
        }

        println!(
            "{:<18} {:<14} {:>6} {:>10} {:>8}  {}",
            "user", "state", "pct", "speed", "size", "file"
        );
        for t in &list {
            println!(
                "{:<18} {:<14} {:>5.1}% {:>10} {:>8}  {}",
                fmt::truncate(&t.username, 18),
                fmt::state(&t.state),
                t.percent_complete,
                fmt::speed(t.average_speed),
                fmt::bytes(t.size),
                fmt::truncate(t.name(), 42),
            );
        }

        if !a.watch || list.iter().all(|t| t.is_done()) {
            let ok = list.iter().filter(|t| t.succeeded()).count();
            let bad = list.iter().filter(|t| t.errored()).count();
            if a.watch {
                println!("\n{ok} succeeded, {bad} failed");
            }
            return Ok(());
        }

        tokio::time::sleep(Duration::from_secs(2)).await;
        println!();
    }
}

async fn browse(api: &Client, a: &BrowseArgs) -> Result<()> {
    eprintln!("browsing {} — this can take a while for large shares", a.username);
    let res = api.browse(&a.username).await?;

    if let Ok(s) = api.user_status(&a.username).await {
        eprintln!("status: {}", s.presence);
    }
    if let Ok(info) = api.user_info(&a.username).await {
        eprintln!(
            "slots: {}  queue: {}  free slot: {}",
            info.upload_slots, info.queue_length, info.has_free_upload_slot
        );
        let desc = info.description.trim();
        if !desc.is_empty() {
            // Peers often state sharing expectations here; worth seeing before
            // you queue a dozen files from them.
            for line in desc.lines().take(4) {
                eprintln!("| {}", fmt::truncate(line, 76));
            }
        }
    }

    let dirs: Vec<_> = res
        .directories
        .iter()
        .filter(|d| match &a.filter {
            Some(f) => d.name.to_lowercase().contains(&f.to_lowercase()),
            None => true,
        })
        .collect();

    println!("{} directories", dirs.len());
    for d in dirs.iter().take(a.limit) {
        println!("{:>5}  {}", d.file_count, d.name);
        if a.files {
            for f in &d.files {
                println!(
                    "         {:>8} {:>6} {:>5}  {}",
                    fmt::bytes(f.size),
                    fmt::bitrate(f.bit_rate),
                    fmt::duration(f.length),
                    models::basename(&f.filename)
                );
            }
        }
    }
    if dirs.len() > a.limit {
        println!("... and {} more", dirs.len() - a.limit);
    }
    Ok(())
}

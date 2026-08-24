# slskd-cli

A terminal client for [slskd](https://github.com/slskd/slskd) — search Soulseek, queue
downloads, and watch transfers without opening a browser.

Ships a single binary, `slsk`, that works as both a scriptable CLI and an interactive TUI.

```
$ slsk get "robbie dupree steal away"
searching "robbie dupree steal away" — 208 responses, 399 files (37 locked)
picked:
#   user               slot    q    speed    size   rate   len  file
1   recovery8655       yes     0  8341KB/s   8.0MB   320k  3:26  461. Robbie Dupree - Steal Away.mp3
queued  461. Robbie Dupree - Steal Away.mp3  from recovery8655
```

## Why

The slskd web UI is good, but reaching for a browser to grab one track is friction, and
picking a source by hand means eyeballing hundreds of near-identical rows. This does the
picking for you and stays in the terminal.

## Install

Requires Rust 1.82 or newer.

```sh
git clone https://github.com/joshjetson/slskd-cli
cd slskd-cli
cargo install --path .
```

## Setup

```sh
slsk init
```

That writes `~/.config/slskd-cli/config.toml`. Point it at your instance:

```toml
api_key_file = "~/.config/slskd-cli/.apikey"
probe_timeout_ms = 1500

[[endpoints]]
name = "lan"
url = "http://192.168.1.50:5030"

[[endpoints]]
name = "remote"
url = "https://slskd.example.com"
```

Endpoints are probed concurrently and the first healthy one wins, with ties broken by
order — so a LAN address and a public hostname for the same server can both be listed and
the fast one is used automatically when you're home.

### API key

Add a key to `slskd.yml` under `web.authentication`:

```yaml
web:
  authentication:
    api_keys:
      slskd_cli:
        key: <at least 16 characters>
        role: readwrite
        cidr: 0.0.0.0/0,::/0
```

`readwrite` is required — `readonly` cannot queue downloads. Restart slskd, then put the
key somewhere the client can read it:

```sh
install -m 600 /dev/null ~/.config/slskd-cli/.apikey
printf '%s' 'your-key-here' > ~/.config/slskd-cli/.apikey
```

`SLSKD_API_KEY` in the environment overrides the file if you'd rather not keep it on disk.

## Usage

```sh
slsk                                  # interactive TUI
slsk status                           # connection, shares, version
slsk search "steely dan peg"          # ranked results, nothing queued
slsk get "toto africa"                # search, pick the best source, confirm, queue
slsk get "toto rosanna" --yes         # skip the prompt
slsk transfers --watch                # follow progress to completion
slsk transfers --clear                # drop completed and errored records
slsk browse someuser --filter jazz    # explore a peer's shares
```

Useful flags on `search` and `get`:

| Flag | Effect |
| --- | --- |
| `--artist NAME` | Require the artist to appear in the path |
| `--ext flac` | Restrict format; repeatable. Defaults to mp3 |
| `--any-format` | Allow any audio format |
| `--min-bitrate 256` | Floor on bitrate, ignored for lossless |
| `--no-duration-check` | Keep results with implausible runtimes |
| `--allow-variants` | Keep remixes, covers, karaoke and live cuts |
| `--count N` | Queue the top N sources instead of one |

### TUI keys

| Key | Action |
| --- | --- |
| `/` | Type a query, `Enter` to run |
| `1` `2` `3`, `Tab` | Switch between search, transfers, browse |
| `j` `k`, arrows | Move; `g` / `G` for top and bottom |
| `d` | Queue the selected file |
| `b` | Browse the selected peer's shares |
| `c` | Clear completed transfers |
| `a` | Toggle mp3-only vs any format |
| `v` | Toggle whether remixes and covers are allowed |
| `r` | Refresh |
| `q` | Quit |

## How sources are ranked

A search for a well-known song returns thousands of files from hundreds of peers, and they
are not interchangeable. Most of the gap between a good download and a bad one is which
peer you pick, not which words you typed. Candidates are filtered, then ordered by:

1. **A free upload slot.** Without one you join a queue that may never move.
2. **A short queue.** Peers advertising hundreds of queued files are effectively offline.
3. **Plausible duration.** The one that catches real mistakes — a search for a 3:26 album
   track cheerfully returns 90-second radio edits, 30-second samples, and hour-long DJ
   mixes that merely mention it. Bitrate will never catch those; runtime does.
4. **Not a remix, cover, or karaoke take.** Duration cannot catch these — a full-length
   remix or a faithful cover has a perfectly ordinary runtime. Variant words are matched
   as whole tokens, so `mix` rejects `(Ken@Work Mix)` without touching a band named
   Mixtapes. The folder is checked too: a filename can look completely innocent while
   sitting inside `A Tribute To Daryl Hall And John Oates`, and everything in there is a
   cover. A variant word is only disqualifying if you didn't ask for it — search for
   `peg live` and live takes stay in.
5. **Bitrate**, capped at 320. Higher figures usually mean a transcode, not a better file.
6. **Upload speed**, last — it only decides how fast an already-good file arrives.

Locked files are dropped, never queued. Use `--no-duration-check` for mixes and medleys,
`--allow-variants` when you actually want the remix.

## Notes on slskd

- Searches are asynchronous. `"Completed, TimedOut"` is the **normal** terminal state — a
  Soulseek search ends when its clock runs out, not when the network is exhausted.
- Soulseek paths are Windows-style and backslash-separated regardless of platform.
- Soulseek rate-limits searches. Firing off dozens in quick succession will get the server
  connection closed on you, after which slskd reconnects with exponential backoff. Pace
  bulk work accordingly.
- slskd 0.22 has no `/hub/transfers` SignalR hub, so transfers are polled rather than
  pushed. At a two-second interval the difference isn't perceptible.

## Development

```sh
cargo test          # ranking, tokenizing, formatting, URL encoding
cargo build --release
```

The ranking rules live in `src/rank.rs` and are covered by tests that encode the actual
failure modes — a radio edit outranking the album cut, a 999kbps mislabel beating an honest
320, a coincidental title match from the wrong artist.

## License

MIT

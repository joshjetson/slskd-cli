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

The TUI:

```
 slsk   1 search  2 transfers  3 browse   sunjet · online · 57 shared
┌ query (mp3) ─────────────────────────────────────────────────────────────────────────────────────────┐
│toto                                                                                                  │
└──────────────────────────────────────────────────────────────────────────────────────────────────────┘
┌ results (3) — d to queue, b to browse peer ──────────────────────────────────────────────────────────┐
│user             slot queue speed   rate   len    size     file                     folder            │
│poly_blooper     yes  0     7812KB/ 320k   4:55   7.6MB    01 - Toto - Africa.mp3   Toto IV           │
│recovery8655     yes  0     7812KB/ 320k   4:55   7.6MB    04 - Toto - Rosanna.mp3  Toto IV           │
│xanshark         yes  0     7812KB/ 320k   4:55   7.6MB    Toto - Georgy Porgy.mp3  Toto IV           │
│                                                                                                      │
│                                                                                                      │
│                                                                                                      │
│                                                                                                      │
│                                                                                                      │
└──────────────────────────────────────────────────────────────────────────────────────────────────────┘
42 matching files from 187 responses (311 locked, filtere q quit · / search · jk move · d queue · b brow
```

## Why

The slskd web UI is good, but reaching for a browser to grab one track is friction, and
picking a source by hand means eyeballing hundreds of near-identical rows. This does the
picking for you and stays in the terminal.

## Install

```sh
git clone https://github.com/joshjetson/slskd-cli
cd slskd-cli
cargo install --path . --locked
```

`--locked` matters. Without it Cargo ignores `Cargo.lock` and re-resolves to the newest
compatible dependencies, several of which now require a newer rustc than the pinned set
does — so a toolchain that builds this repo fine will fail to `cargo install` it.

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

## Switching networks

Auto-detection handles this on its own: off the LAN, the local address fails its health
check and the public one is used. Nothing to edit.

The catch is that probing costs whatever the unreachable endpoint takes to time out. When
you already know where you are, name the endpoint and skip probing entirely:

```sh
slsk status                              # probe both      ~1.8s off-network
SLSKD_ENDPOINT=remote slsk status        # no probe        ~0.2s
```

Export it in a shell profile to pin a machine to one path, or set it per-command when
travelling. Nothing in the config file needs to change.

## Environment

| Variable | Effect |
| --- | --- |
| `SLSKD_URL` | Use this endpoint, ignoring the config's endpoint list entirely |
| `SLSKD_ENDPOINT` | Use the configured endpoint with this `name` (e.g. `lan`, `remote`) |
| `SLSKD_API_KEY` | API key, overriding both `api_key` and `api_key_file` |

Endpoint selection resolves most-explicit-first: `--url`, then `SLSKD_URL`, then
`--endpoint`, then `SLSKD_ENDPOINT`, then probing. An unknown name is an error that lists
what is configured, rather than a silent fallback.

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
See [Environment](#environment) for the other variables.

## Usage

```sh
slsk                                  # interactive TUI
slsk status                           # connection, shares, version
slsk search "steely dan peg"          # ranked results, nothing queued
slsk get "toto africa"                # search, pick the best source, confirm, queue
slsk search "this is it" --artist "kenny loggins"   # separate the two, see below
slsk get "toto rosanna" --yes         # skip the prompt
slsk transfers --watch                # follow progress to completion
slsk transfers --clear                # drop completed and errored records
slsk browse someuser --filter jazz    # explore a peer's shares

slsk --endpoint remote status         # force a configured endpoint
slsk --url http://host:5030 status    # bypass config entirely
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
| `--tries N` | Distinct peers to try before giving up (default 5) |

### Seeing what you already have

The search tab carries a `dl` column so you never have to leave it to find out what a
keypress did:

| Shows | Meaning |
| --- | --- |
| *(blank)* | not queued |
| `queued` | accepted by slskd, waiting on the peer |
| `42%` | downloading now |
| `done` | finished |
| `failed` | the peer dropped it |
| `have` | a file of that name already came down **from someone else** |

`have` is the one that saves you time — it stops you queueing the same song again from a
different peer just because the first attempt scrolled out of view.

Transfers are listed newest first, ordered by when you requested them, because slskd
returns them grouped by peer, which tracks nothing you did.

### TUI keys

| Key | Action |
| --- | --- |
| `/` | Type a query, `Enter` to run |
| `1` `2` `3`, `Tab` | Switch between search, transfers, browse |
| `j` `k`, arrows | Move; `g` / `G` for top and bottom |
| `d` | Queue the selected file |
| `x` | Remove the selected transfer (transfers tab) |
| `b` | Browse the selected peer's shares |
| `c` | Clear finished transfers, or clear the browse listing |
| `a` | Toggle mp3-only vs any format |
| `v` | Toggle whether remixes and covers are allowed |
| `r` | Refresh |
| `Esc` | Cancel a running search |
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
5. **The title, matched where the title actually is.** Peers name files
   `Artist - Album - NN - Title.ext`, which puts the album name in the same string the
   title is tested against — searching the Doobie Brothers' *Minute By Minute* happily
   returns `... - Minute by Minute - 01 - Here to Love You.mp3`, right album, wrong song.
   Only the final segment is matched. And a title made of short words survives
   tokenising as almost nothing — *This Is It* reduces to `this`, which matches *This Is
   How My Song Goes* — so weak titles must additionally appear as a contiguous phrase.
6. **Bitrate**, capped at 320. Higher figures usually mean a transcode, not a better file.
7. **Upload speed**, last — it only decides how fast an already-good file arrives.

Prefer `--artist` over folding the artist into the query. `slsk search "kenny loggins this
is it"` treats every word as part of the title, which makes a weak title look strong and
lets `with_this_ring.mp3` through. `slsk search "this is it" --artist "kenny loggins"`
narrowed the same 4062 files to the 2 correct ones.

If a peer refuses the transfer — `Transfer rejected: Too many megabytes`, a full queue,
or having gone offline since the search — `get` moves to the next source rather than
failing. Retrying the same peer cannot help; the next one down usually works.

Locked files are dropped, never queued. Use `--no-duration-check` for mixes and medleys,
`--allow-variants` when you actually want the remix.

## Notes on slskd

- Searches are asynchronous. `"Completed, TimedOut"` is the **normal** terminal state — a
  Soulseek search ends when its clock runs out, not when the network is exhausted.
- **Results only exist once a search finishes.** While a search is `InProgress`, slskd
  increments `responseCount` live but leaves the `responses` array empty, filling it in
  only at the terminal state. Reading early gives you a search that claims 150 responses
  and carries none, which looks exactly like a search that matched nothing. Typical
  completion is ~35s, so `--wait` defaults to 60 and the client refuses to report results
  from a search that hasn't finished rather than reporting a false empty.
- Soulseek paths are Windows-style and backslash-separated regardless of platform.
- Soulseek rate-limits searches. Firing off dozens in quick succession gets the server
  connection closed on you; slskd then reconnects with exponential backoff, and further
  login attempts while throttled prolong it. Pace bulk work at roughly one search every
  20-30 seconds. When this happens slskd answers searches with a bare `500`, so the client
  checks the connection and reports what is actually wrong.
- slskd 0.22 has no `/hub/transfers` SignalR hub, so transfers are polled rather than
  pushed. At a two-second interval the difference isn't perceptible.
- Searches take roughly 35 seconds, so the TUI runs them on a background task: the
  interface keeps drawing, keys keep working, `Esc` cancels, and a spinner shows elapsed
  time. Nothing about a search blocks the frame loop.

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

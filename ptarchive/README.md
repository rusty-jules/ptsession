# ptarchive

A tool for archiving [Pro Tools][pro-tools] sessions as [Oras][oras][^1] artifacts.

## Features

- Configurable, per-file compression
- Archival and retrieval of original file creation and modified timestamps
- Fast file searching akin to Pro Tools' "Find by name and Match Duration" setting thanks to [`ignore`][ignore]
- Optional skipping files that don't have any regions in the session
- Local and remote Pro Tools session parsing
- Multi-threaded, multi-connection uploads and downloads
- Local caching of audio file digests for fast iterative archiving of multiple session versions
- Configuration via cli arguments or config file
- Optional metrics and json logs

All without ever opening Pro Tools.

## Why not just a zip file?

Many engineers zip session folders as a delivery method. This is a perfectly valid, tried-and-true method for sharing
sessions with collaborators. For the purposes of archival, it poses some challenges though:

1. How can you add a new version of a session (and any additional audio files) to a zip file you already sent?
2. How can you determine which audio files someone else may already have?
3. How do you stop files from being duplicated across zip files?
4. How do you know what's already in potentially hundreds of zip files you have created?

Many workarounds to these problems involve the venerated "Save Copy In" + "Compact" options in Pro Tools, but
these are error prone and potentially dangerous operations (how many times have you double checked the session name
before clicking the `compact` button?).

Luckily, these problems have all been solved by [content-addressable storage][cas], which modern oci repositories lean
on heavily.
This is great for archival, and even for collaboration should you choose. OCI repositories have strong notions of
authentication and access control, allowing you to safely share sessions iteratively instead of in a one-off fashion.
This might even be more desirable than continous sharing via the likes of Dropbox, Google Drive, or OneDrive, especially
when sharing sessions with a client instead of a co-worker.

## Why OCI as a storage backend?

Containers have taken the cloud infrastructure industry by storm and are still under active and heavy development with
no end in sight.
The [Open Container Initiative][oci] (OCI) has done a lot of work to standardize their interfaces and storage backends.
By framing more and more types of "versioned file + dependencies" as image layers (just like other not-a-container
oriented systems such as [`brew`][brew-oci] have been doing) we can plug in to this massive ecosystem.

When pointed at an OCI compliant registry, such as [zot][zot], you garner many of the benefits that the container storage
industry as worked hard on to store massive amounts of versioned files at scale, such as:

- natural session versioning by framing repositories as sessions and image tags as session versions
- audio file deduplication across both session versions and all archived sessions within a registry
- graphql apis that essentially act as a database for custom metadata
- the oci `referrers` api for storing large files that are relevant to, but not necessarily a part of, a session
- potentially "infinite" storage space when using an s3-like object storage backend for your registry
- progressive garbage collection of unreferenced files
- the many other (growing) benefits like storage tiering, repository replication, mirroring, proxying, and more as
  container registries are continously developed and improved upon

## Example Configuration

```toml
# ~/.ptarchive/config.toml
[logs]
level = "trace"
format = "json"

[logs.file]
directory = "~/.ptarchive/logs"
filename.method = "hash"

[metrics]
flavor = "influxdb"
endpoint = "http://100.103.172.27:30889/api/v2/write"
database = "ptarchive"
table = "metrics"

[command.push]
compression = { compressor = "zstd", level = 9 }
depth = 4
parallelism = 8 
regions-only = true
search-paths = ["../"]

[cache]
path = "~/.ptarchive/cache.db"
```

> [!NOTE]
> Currently values in the config file override any values passed via flags.

[^1]: Currently ptarchive is more "oras-like". Though oras can pull any session pushed by ptarchive, oras does not support
any compression algorithm other than gzip, which ptarchive does not use by default. This results in files being pulled
without being decompressed and without the correct extension for later decompression. ptarchive transparently compresses
and decompresses on push and pull.

[pro-tools]: https://www.avid.com/pro-tools
[oras]: https://github.com/oras-project/oras
[ignore]: https://github.com/BurntSushi/ripgrep/tree/master/crates/ignore
[cas]: https://en.wikipedia.org/wiki/Content-addressable_storage#:~:text=Content%2Daddressable%20storage%20is%20similar,physical%20storage%20of%20the%20content.
[oci]: https://opencontainers.org/
[zot]: https://zotregistry.dev
[brew-oci]: https://github.com/orgs/Homebrew/discussions/4335#discussioncomment-5353698

## TODO

- [x] Check digest of files before upload to skip existing blobs
- [ ] Check digest on download to skip existing blobs (avoid FileExists errors)
- [x] File search with tagging cache, parallelism, and reporting (split off with filter)
- [ ] File search depth argument (-B num directories above, -A num dirs below, -C num dirs around (above and below))
- [x] Consider defaulting to zstd -7 (faster than xz, xz may [have issues for archival][xz-issues])
- [x] Non-TUI progress (json/text logging)
- [x] Save date created/modified annotations for sessions & files
- [ ] Compress ptx session files
- [ ] Upload pro tools originiator id's as annotations for finding files in other sessions
- [ ] Better errors (like could not find file)
- [ ] Upload / Download retries ([reqwest_retry_middleware][middleware])
- [ ] Pull caching for downloading the minimum amount of files across downloaded sessions (tricky)

[xz-issues]: https://www.nongnu.org/lzip/xz_inadequate.html
[middleware]: https://docs.rs/reqwest-retry/latest/reqwest_retry/

## Nice to Haves

- [ ] Additive pushing function for finding files with multiple methods
  - When --additive is passed, the manifest is first fetched and all audio files in there will not be
    pushed or searched for. Any files not in the manifest but in the ptx session will be searched for
    with the passed searching paths/methods, and their blobs will be _appended_ to the existing manifest
  - Also allows for a ptsession-daemon that additively adds files to a manifest in the background, and only
    creates a new manifest when a new session file is saved
- [ ] Check memory usage (drop encoder / buffers inside of futures? Size used of Compressor enum?)
- [x] Wrap encoders in Compressor Enum
- [ ] Options for Auth / Https
- [x] Update PtSession to read from `AsyncRead` instead of Path
- [x] Add text, text-extended, table, and json outputs for info
  - [x] session name
  - [x] sample rate
  - [x] version
  - [ ] largest file
  - [ ] compressed/uncompressed session size
  - [ ] in future: plugins, track names

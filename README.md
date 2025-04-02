# ptsession

[Pro Tools] session parser. A Rust rewrite of the amazing work done on [ptformat].

## Features

All features of [ptformat] except for MIDI parsing are supported. Support for Marker parsing and Serialization (thanks to [serde]) has been added.

[Pro Tools]: https://avid.com/pro-tools
[ptformat]: https://github.com/zamaudio/ptformat
[serde]: https://github.com/serde-rs/serde

## TODO

- [ ] Check digest of files before upload to skip existing blobs
- [ ] Check digest on download to skip existing blobs (avoid FileExists errors)
- [ ] File search with tagging cache, parallelism, and reporting (split off with filter)
- [ ] File search depth argument (-B num directories above, -A num dirs below, -C num dirs around (above and below))
- [ ] Non-TUI progress (json/text logging)
- [ ] Upload / Download retries ([reqwest_retry_middleware][middleware])
- [ ] Better errors (like could not find file)
- [ ] Save date created/modified annotations for sessions & files
- [ ] Compress sessions
- [ ] Upload pro tools originiator id's as annotations for finding files in other sessions

[middleware]: https://docs.rs/reqwest-retry/latest/reqwest_retry/

## Nice to Haves
- [ ] Additive pushing function for finding files with multiple methods
  - When --additive is passed, the manifest is first fetched and all audio files in there will not be
    pushed or searched for. Any files not in the manifest but in the ptx session will be searched for
    with the passed searching paths/methods, and their blobs will be _appended_ to the existing manifest
  - Also allows for ptsession-daemon that additively adds files to a manifest in the background, and only
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

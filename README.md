<p align="center">
  <h1 align="center">mp4forge</h1>
  <p align="center">
    Rust library and CLI for inspecting, extracting, probing, and rewriting MP4 box structures.
  </p>
  <p align="center">
    <a href="https://crates.io/crates/mp4forge"><img src="https://img.shields.io/crates/v/mp4forge.svg" alt="Crates.io"></a>
    &nbsp;&nbsp;
    <a href="https://docs.rs/mp4forge"><img src="https://img.shields.io/docsrs/mp4forge" alt="docs.rs"></a>
    &nbsp;&nbsp;
    <a href="LICENSE-MIT"><img src="https://img.shields.io/crates/l/mp4forge.svg" alt="License"></a>
    &nbsp;&nbsp;
    <img src="https://img.shields.io/badge/MSRV-1.88-blue.svg" alt="MSRV 1.88">
  </p>
</p>

---

- Typed MP4 and ISOBMFF box model with registry-backed custom box support
- Low-level traversal, extraction, stringify, probe, and writer APIs
- Thin typed path-based helpers for common extraction and rewrite flows
- Built-in CLI for `dump`, `extract`, `probe`, `psshdump`, `edit`, and `divide`
- Shared-fixture coverage for regular MP4, fragmented MP4, encrypted init segments, and QuickTime-style metadata cases

## Installation

```toml
[dependencies]
mp4forge = "0.2.0"
```

Install the CLI from crates.io:

```sh
cargo install mp4forge --locked
```

Install the current checkout locally:

```sh
cargo install --path . --locked
```

The published crate includes both the library and the `mp4forge` binary from `src/bin/mp4forge.rs`.

## CLI

```text
USAGE: mp4forge COMMAND [ARGS]

COMMAND:
  divide       split a fragmented MP4 into track playlists
  dump         display the MP4 box tree
  edit         rewrite selected boxes
  extract      extract raw boxes by type
  psshdump     summarize pssh boxes
  probe        summarize an MP4 file
```

For example:

```sh
mp4forge dump input.mp4
mp4forge probe input.mp4
mp4forge psshdump encrypted_init.mp4
```

## Feature Flags

`mp4forge` currently ships without public Cargo feature flags.

> See the [`examples/`](./examples) directory for both the low-level and high-level public API story, including typed extraction in `extract_track_ids_typed.rs`, typed rewrite in `rewrite_emsg.rs`, structure walking, probing, writer-backed rewrite, and custom box registration.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT) at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in mp4forge by you, as defined in the Apache-2.0 license, shall be dual licensed as above, without any additional terms or conditions.

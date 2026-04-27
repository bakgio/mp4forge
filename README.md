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
- Thin typed path-based helpers and byte-slice convenience wrappers for common extraction, rewrite, and probe flows
- Fragmented top-level `sidx` analysis, planning, and rewrite APIs for supported layouts
- Feature-gated decryption APIs and a sync-only `decrypt` CLI for the supported protected MP4 families
- Built-in CLI for `decrypt`, `dump`, `extract`, `probe`, `psshdump`, `edit`, and `divide`
- Shared-fixture coverage for regular MP4, fragmented MP4, encrypted init segments, QuickTime-style metadata cases, and derived real codec fixtures for additional codec-family coverage

## Installation

```toml
[dependencies]
mp4forge = "0.6.0"

# With optional features:
# mp4forge = { version = "0.6.0", features = ["async"] }
# mp4forge = { version = "0.6.0", features = ["decrypt"] }
# mp4forge = { version = "0.6.0", features = ["decrypt", "async"] }
# mp4forge = { version = "0.6.0", features = ["serde"] }
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

## Feature Flags

`mp4forge` keeps the default dependency surface minimal and currently exposes these optional public
feature flags:

- `async`: enables the additive library-side async I/O surface for seekable readers and writers.
  This rollout is Tokio-based, expects a Tokio runtime in the caller, targets seekable
  `AsyncRead + AsyncSeek` and `AsyncWrite + AsyncSeek` inputs and outputs, supports normal
  multithreaded `tokio::spawn` usage for the supported library paths, and keeps the current CLI on
  the existing sync path.
- `decrypt`: enables the additive decryption input, progress, and support-matrix types that fix
  the public shape for the decryption surface while keeping the default build unchanged. The
  landed sync library path covers the Common Encryption family (`cenc`, `cens`, `cbc1`, `cbcs`),
  PIFF-triggered compatibility behavior, OMA DCF atom files and protected movie layouts, Marlin
  IPMP ACBC and ACGK OD-track movies, and the retained IAEC protected-movie path. When combined
  with `async`, it also enables the additive file-backed Tokio async decrypt companions, while the
  CLI remains on the synchronous path.
- `serde`: derives `Serialize` and `Deserialize` for the reusable public report structs under
  `mp4forge::cli::probe` and `mp4forge::cli::dump`, along with their nested public codec-detail,
  media-characteristics, `FieldValue`, and `FourCc` data. This is intended for library-side report
  embedding and uses the Rust field names of those public structs; the CLI `-format` outputs keep
  their existing hand-authored JSON and YAML schemas.

## CLI

```text
USAGE: mp4forge COMMAND [ARGS]

COMMAND:
  decrypt      decrypt a protected MP4 file
  divide       split a fragmented MP4 into track playlists
  dump         display the MP4 box tree
  edit         rewrite selected boxes
  extract      extract raw boxes by type or path
  psshdump     summarize pssh boxes
  probe        summarize an MP4 file
```

`decrypt` is available when the crate is built with `--features decrypt`. The CLI stays
sync-only, accepts repeated `--key ID:KEY`, optional `--fragments-info FILE`, and optional
`--show-progress`, and reuses the same library decryption surface that backs the feature-gated
sync and async APIs.

`divide` currently targets fragmented inputs with up to one AVC video track and one MP4A audio
track, including encrypted wrappers that preserve those original sample-entry formats. Pass
`-validate` when you want the same probe-driven layout checks without creating any output files.

`dump` defaults to the existing human-readable tree view. Pass `-format json` or `-format yaml` for
deterministic structured tree export with stable `payload_fields` for supported boxes; `-full` and
`-a` still control when large raw or unsupported payloads expand beyond the default summary-oriented
view. Add repeatable `-path <box/path>` filters when you want text or structured output rooted at
only the matched parsed subtrees instead of the whole file.

`edit` keeps the existing global `tfdt` replacement and `-drop` behavior, and now also accepts
repeatable `-path` filters when you want `-base_media_decode_time` to target only matching parsed
box paths.

`psshdump` defaults to the existing human-readable protection summary. Pass `-format json` or
`-format yaml` for deterministic structured reports with box offsets, system IDs, KIDs, `Data`
bytes, and the legacy raw-box base64 payload. Add repeatable `-path <box/path>`, `-system-id
<uuid>`, or `-kid <uuid>` filters when you want text and structured reports to return only the
matching protection boxes.

`probe` defaults to structured JSON output. When the input carries parsed codec-configuration
boxes, the report now includes a nested `codec_details` object per track for families such as AVC,
HEVC, AV1, VP8/VP9, MP4A, Opus, AC-3, PCM, XML subtitles, text subtitles, and WebVTT. When sample
entries carry `btrt`, `colr`, `pasp`, or `fiel`, the richer CLI path also emits nested
`media_characteristics` data such as declared bitrate, colorimetry, pixel aspect ratio, and
field-order hints. Pass `-detail light` for a lighter-weight probe that skips per-sample,
per-chunk, bitrate, and IDR aggregation, or use `mp4forge::probe::ProbeOptions` from the library
when you need the same control programmatically.

> See the [`examples/`](./examples) directory for the crate's low-level and high-level API usage
> patterns, including the feature-gated decrypt example and the Tokio-based async library example
> behind the optional `async` feature.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT) at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in mp4forge by you, as defined in the Apache-2.0 license, shall be dual licensed as above, without any additional terms or conditions.

# 0.5.0 (April 25, 2026)

- Added first-class encrypted metadata coverage for typed `senc`, typed `sgpd(seig)`, resolved sample-encryption helpers, and broader encrypted fragmented fixture coverage across extraction, rewrite, and probe flows
- Added additive top-level `sidx` analysis, planning, rewrite, documentation, and example support for the supported fragmented-file layouts
- Expanded typed box coverage across fragmented timing, metadata, and codec families, including `clap`, `SmDm`, `CoLL`, `dec3`, `dac4`, `vvcC`, AVS3, FLAC, MPEG-H, `subs`, `elng`, `ssix`, `leva`, `evte`, `silb`, `emib`, `emeb`, `ID32`, loudness boxes, `prft`, typed `tref` children, `sthd`, `nmhd`, `kind`, `mime`, `cdat`, and selected legacy `uuid` payloads
- Improved low-level robustness by preserving legal trailing bytes in `VisualSampleEntry` layouts and carrying those bytes cleanly through traversal and rewrite paths
- Added `prft` timestamp and flag helpers, richer examples, and broader regression coverage for fragmented, encrypted, metadata-rich, and legacy MP4 layouts

# 0.4.0 (April 22, 2026)

- Added richer additive probe surfaces for broader codec families, codec-specific details, media-characteristics reporting, and lighter-weight probe controls for large-file inspection
- Added deterministic structured dump and `psshdump` JSON/YAML export, field-level dump payload reporting, and repeatable path or protection filters shared across text and structured output
- Expanded CLI path ergonomics with parsed-path extraction, subtree-scoped dump selection, path-scoped typed edit flows, and richer `psshdump` filtering by box path, system ID, and KID
- Improved `divide` by deriving playlist signaling from probed metadata and adding a first-class validation mode for unsupported fragmented layouts before any output is written
- Added optional `serde` support for reusable report types, including nested probe and dump companion data intended for library-side embedding
- Expanded checked-in fixture coverage for AV1, VP9, AAC, Opus, and PCM, and added dedicated high-level fuzz targets for probe, structured dump, and typed rewrite surfaces
- Refined README guidance, examples, tests, and goldens across the newer higher-level library and CLI workflows while preserving the existing low-level usage paths

# 0.3.0 (April 22, 2026)

- Added byte-slice convenience helpers for typed extract, rewrite, and probe workflows so higher-level integrations can stay in-memory without dropping to the lower-level APIs
- Added exact raw box-byte extraction helpers for full-box and payload-only reads, including registry-aware variants for custom box decoding workflows
- Added additive `BoxPath` string parsing with `BoxPath::parse`, `FromStr`, and `TryFrom<&str>` so ergonomic path construction can build on the existing low-level API
- Expanded examples, tests, and comparison coverage around the new ergonomic helpers while preserving the existing low-level usage paths
- Refined public docs and README guidance for the new helper surface

# 0.2.0 (April 21, 2026)

- Added typed path-based extraction helpers for common read flows: `extract_box_as`, `extract_boxes_as`, and `extract_boxes_as_with_registry`
- Added typed path-based rewrite helpers for common edit flows: `rewrite_box_as`, `rewrite_boxes_as`, and `rewrite_boxes_as_with_registry`
- Improved matched payload diagnostics so extraction and rewrite failures report the path, box type, and byte offset that triggered the error
- Added higher-level examples for the ergonomic helper layer while preserving the existing low-level examples
- Polished public docs, README coverage, packaging metadata, and release validation around the new helper surface

# 0.1.0 (April 21, 2026)

- Initial crate release

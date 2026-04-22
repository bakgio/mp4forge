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

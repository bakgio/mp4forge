# 0.2.0 (April 21, 2026)

- Added typed path-based extraction helpers for common read flows: `extract_box_as`, `extract_boxes_as`, and `extract_boxes_as_with_registry`
- Added typed path-based rewrite helpers for common edit flows: `rewrite_box_as`, `rewrite_boxes_as`, and `rewrite_boxes_as_with_registry`
- Improved matched payload diagnostics so extraction and rewrite failures report the path, box type, and byte offset that triggered the error
- Added higher-level examples for the ergonomic helper layer while preserving the existing low-level examples
- Polished public docs, README coverage, packaging metadata, and release validation around the new helper surface

# 0.1.0 (April 21, 2026)

- Initial crate release

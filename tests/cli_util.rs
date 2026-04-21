use mp4forge::cli::util::should_have_no_children;

use crate::support::fourcc;

mod support;

#[test]
fn helper_marks_known_leaf_boxes_without_claiming_containers() {
    assert!(should_have_no_children(fourcc("ftyp")));
    assert!(should_have_no_children(fourcc("stco")));
    assert!(should_have_no_children(fourcc("trun")));
    assert!(!should_have_no_children(fourcc("moov")));
    assert!(!should_have_no_children(fourcc("trak")));
    assert!(!should_have_no_children(fourcc("meta")));
}

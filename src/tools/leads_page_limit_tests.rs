use super::*;

#[test]
fn limited_by_names_the_cause_for_every_combination() {
    let size = |at_ceiling| PageSize {
        bytes: 10,
        at_ceiling,
    };
    for stopped_by in [None, Some(PageLimit::Limit), Some(PageLimit::PageBytes)] {
        for at_ceiling in [false, true] {
            assert_eq!(
                limited_by(true, stopped_by, size(at_ceiling)),
                PageLimit::None,
                "a complete page has no limit ({stopped_by:?}, {at_ceiling})"
            );
        }
    }
    for at_ceiling in [false, true] {
        assert_eq!(
            limited_by(false, Some(PageLimit::Limit), size(at_ceiling)),
            PageLimit::Limit
        );
    }
    for stopped_by in [None, Some(PageLimit::PageBytes)] {
        assert_eq!(
            limited_by(false, stopped_by, size(false)),
            PageLimit::PageBytes
        );
        assert_eq!(
            limited_by(false, stopped_by, size(true)),
            PageLimit::Ceiling
        );
    }
}

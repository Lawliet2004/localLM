use glib::prelude::*;

#[test]
fn optimized_string_iteration_preserves_front_back_and_skips() {
    let strings = ["alpha", "βeta", "gamma", "delta", "終"];
    let variant = strings.to_variant();
    for _ in 0..1000 {
        assert_eq!(variant.array_iter_str().unwrap().collect::<Vec<_>>(), strings);
        assert_eq!(variant.array_iter_str().unwrap().rev().collect::<Vec<_>>(), strings.iter().rev().copied().collect::<Vec<_>>());
        assert_eq!(variant.array_iter_str().unwrap().last(), Some("終"));
        let mut iter = variant.array_iter_str().unwrap();
        assert_eq!(iter.next(), Some("alpha"));
        assert_eq!(iter.next_back(), Some("終"));
        assert_eq!(iter.nth(1), Some("gamma"));
        assert_eq!(iter.nth_back(0), Some("delta"));
        assert_eq!(iter.next(), None);
        assert_eq!(iter.next_back(), None);
    }
}

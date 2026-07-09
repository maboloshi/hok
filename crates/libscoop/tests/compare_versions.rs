use libscoop::compare_versions;
use std::cmp::Ordering::{Equal, Greater, Less};

#[test]
fn test_equal_versions() {
    assert_eq!(compare_versions("1.0.0", "1.0.0"), Equal);
    assert_eq!(compare_versions("2.5.10", "2.5.10"), Equal);
    assert_eq!(compare_versions("0.0.1", "0.0.1"), Equal);
}

#[test]
fn test_v_prefix() {
    assert_eq!(compare_versions("v1.0.0", "1.0.0"), Equal);
    assert_eq!(compare_versions("1.0.0", "v1.0.0"), Equal);
    assert_eq!(compare_versions("V2.0", "2.0"), Equal);
    assert_eq!(compare_versions("v1.0", "v1.0"), Equal);
}

#[test]
fn test_numeric_segments() {
    assert_eq!(compare_versions("2.0.0", "1.9.9"), Greater);
    assert_eq!(compare_versions("1.9.9", "2.0.0"), Less);
    assert_eq!(compare_versions("1.0.1", "1.0.0"), Greater);
    assert_eq!(compare_versions("10.0", "9.0"), Greater);
}

#[test]
fn test_different_length() {
    assert_eq!(compare_versions("1.0", "1.0.0"), Equal);
    assert_eq!(compare_versions("1.0.1", "1.0"), Greater);
    assert_eq!(compare_versions("1.0", "1.0.1"), Less);
}

#[test]
fn test_pre_release() {
    assert_eq!(compare_versions("1.0.0-rc1", "1.0.0"), Less);
    assert_eq!(compare_versions("1.0.0", "1.0.0-rc1"), Greater);
    assert_eq!(compare_versions("1.0.0-alpha", "1.0.0-beta"), Less);
    assert_eq!(compare_versions("1.0.0-beta", "1.0.0-alpha"), Greater);
}

#[test]
fn test_text_segments() {
    assert_eq!(compare_versions("1.0.0-beta", "1.0.0-alpha"), Greater);
    assert_eq!(compare_versions("1.0.0-rc", "1.0.0-beta"), Greater);
}

#[test]
fn test_mixed_separators() {
    assert_eq!(compare_versions("1.0.0_beta", "1.0.0-alpha"), Greater);
}

#[test]
fn test_edge_cases() {
    // Zero padding
    assert_eq!(compare_versions("1.00.0", "1.0.0"), Equal);
    // Single segment
    assert_eq!(compare_versions("5", "4"), Greater);
    assert_eq!(compare_versions("4", "5"), Less);
    // Same major different minor
    assert_eq!(compare_versions("3.2", "3.10"), Less);
    // Empty segments
    assert_eq!(compare_versions("1.0.0-", "1.0.0"), Less);
}

#[test]
fn test_nightly() {
    assert_eq!(compare_versions("nightly", "nightly"), Equal);
    assert_eq!(compare_versions("nightly", "1.0.0"), Less); // text < num
}

//! The polling rule (plan "The polling rule"): only identical consecutive accesses merge.

mod common;
use common::collapse_polls;

fn v(lines: &[&str]) -> Vec<String> {
    lines.iter().map(|s| s.to_string()).collect()
}

#[test]
fn runs_of_different_length_compare_equal() {
    let a = v(&[
        "IO read.b $fffffc00 = $02 pc=fc4856",
        "IO read.b $fffffc00 = $02 pc=fc4856",
        "IO read.b $fffffc00 = $83 pc=fc4856",
    ]);
    let b = v(&[
        "IO read.b $fffffc00 = $02 pc=fc4856",
        "IO read.b $fffffc00 = $02 pc=fc4856",
        "IO read.b $fffffc00 = $02 pc=fc4856",
        "IO read.b $fffffc00 = $02 pc=fc4856",
        "IO read.b $fffffc00 = $83 pc=fc4856",
    ]);
    assert_eq!(collapse_polls(&a), collapse_polls(&b));
}

#[test]
fn a_changed_value_inside_a_run_is_kept() {
    let a = v(&[
        "IO read.b $fffffc00 = $02 pc=fc4856",
        "IO read.b $fffffc00 = $02 pc=fc4856",
        "IO read.b $fffffc00 = $83 pc=fc4856",
    ]);
    assert_eq!(
        collapse_polls(&a),
        v(&[
            "IO read.b $fffffc00 = $02 pc=fc4856",
            "IO read.b $fffffc00 = $83 pc=fc4856"
        ])
    );
}

#[test]
fn order_is_never_changed() {
    let a = v(&["A", "B", "A", "B"]);
    assert_eq!(collapse_polls(&a), a);
}

#[test]
fn different_pcs_are_not_merged() {
    let a = v(&[
        "IO read.b $fffffa01 = $b1 pc=fc62b8",
        "IO read.b $fffffa01 = $b1 pc=fc62c0",
    ]);
    assert_eq!(collapse_polls(&a), a);
    assert_ne!(
        collapse_polls(&a),
        collapse_polls(&a[..1]),
        "a different PC is a real difference"
    );
}

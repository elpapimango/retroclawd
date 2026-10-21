//! The polling rule (`common::collapse_polls`): a line repeated within the last few kept lines merges; anything
//! else still shows.

mod common;
use common::{collapse_polls, comparable};

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
    let a = v(&["A", "B", "C", "D", "E", "A", "B"]);
    assert_eq!(collapse_polls(&a), a, "beyond the window a repeat is kept");
    assert_ne!(
        collapse_polls(&v(&["A", "B", "C"])),
        collapse_polls(&v(&["B", "A", "C"]))
    );
}

/// A polling loop interleaved with a timer interrupt's end-of-interrupt write, a different number of times.
#[test]
fn a_poll_interleaved_with_an_interrupt_merges() {
    let poll = "IO read.b $fffffc00 = $02 pc=fc4856";
    let eoi = "IO write.b $fffffa11 = $df pc=fc050c";
    let next = "IO write.b $fffffc04 = $03 pc=fc63c2";
    let a = v(&[poll, eoi, poll, eoi, next]);
    let b = v(&[poll, eoi, poll, eoi, poll, next]);
    assert_eq!(collapse_polls(&a), collapse_polls(&b));
    assert_eq!(collapse_polls(&a), v(&[poll, eoi, next]));
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

/// A running timer's data register reads whatever the count is at that cycle: the value is masked, the access kept.
#[test]
fn timer_data_reads_are_masked() {
    let a = v(&[
        "IO read.b $fffffa25 = $01 pc=fc77c2",
        "IO write.b $fffffa25 = $00 pc=fc77c2",
    ]);
    let b = v(&[
        "IO read.b $fffffa25 = $02 pc=fc77c2",
        "IO write.b $fffffa25 = $00 pc=fc77c2",
    ]);
    assert_eq!(comparable(&a), comparable(&b));
    assert_eq!(comparable(&a)[0], "IO read.b $fffffa25 = $?? pc=fc77c2");
    let c = v(&["IO read.b $fffffa27 = $01 pc=fc77c2"]);
    let d = v(&["IO read.b $fffffa27 = $02 pc=fc77c2"]);
    assert_ne!(
        comparable(&c),
        comparable(&d),
        "other registers keep their value"
    );
}

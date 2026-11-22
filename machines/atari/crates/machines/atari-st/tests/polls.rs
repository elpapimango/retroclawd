//! The polling rule (`common::collapse_polls`): a line repeated within the last few kept lines merges; anything
//! else still shows.

mod common;
use common::{
    assert_same_ordered_sequences, assert_same_sets, collapse_polls, comparable, streams,
};

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

/// The documented `$FF8604` conflict: the upper byte of a read is masked, the FDC's byte compared.
#[test]
fn ff8604_upper_byte_is_masked() {
    let a = v(&["IO read.w $ffff8604 = $ffe4 pc=fc38b6"]);
    let b = v(&["IO read.w $ffff8604 = $00e4 pc=fc38b6"]);
    let c = v(&["IO read.w $ffff8604 = $00c4 pc=fc38b6"]);
    assert_eq!(comparable(&a), comparable(&b));
    assert_ne!(comparable(&b), comparable(&c));
}

/// An interrupt handler's access between two polls of the main program does not change either stream.
#[test]
fn streams_ignore_interleaving_but_not_order_within_a_pc() {
    let a = v(&["A pc=1", "I pc=9", "B pc=1"]);
    let b = v(&["A pc=1", "B pc=1", "I pc=9"]);
    assert_eq!(streams(&a), streams(&b));
    let c = v(&["B pc=1", "A pc=1", "I pc=9"]);
    assert_ne!(streams(&a), streams(&c));
}

/// Hatari prints the CPU's effective address; the bus has 24 bits, so `$00ff8240` and `$ffff8240` are one register.
#[test]
fn the_top_address_byte_is_not_compared() {
    let a = v(&["IO write.w $00ff8240 = $0123 pc=1022"]);
    let b = v(&["IO write.w $ffff8240 = $0123 pc=1022"]);
    assert_eq!(comparable(&a), comparable(&b));
    assert_eq!(streams(&a), streams(&b));
}

/// The same accesses at each location in another order pass; a different value, or a location only one log has,
/// does not.
#[test]
fn sets_ignore_order_at_a_location_but_not_values() {
    let a = v(&["W $25 pc=1", "W $23 pc=1", "R pc=2"]);
    let b = v(&["W $23 pc=1", "W $25 pc=1", "R pc=2"]);
    assert_same_sets(&a, &b, "order");
    let c = v(&["W $23 pc=1", "W $27 pc=1", "R pc=2"]);
    assert!(std::panic::catch_unwind(|| assert_same_sets(&a, &c, "value")).is_err());
    let d = v(&["W $25 pc=1", "W $23 pc=1"]);
    assert!(std::panic::catch_unwind(|| assert_same_sets(&a, &d, "missing")).is_err());
}

/// The long-run set comparison does not compare GPIP I5 or the FDC's index bit; other bits still count.
#[test]
fn sets_mask_the_transient_bits() {
    assert_same_sets(
        &v(&[
            "IO read.b $fffffa01 = $91 pc=fc69be",
            "IO read.w $ffff8604 = $ff26 pc=fc38b6",
        ]),
        &v(&[
            "IO read.b $fffffa01 = $b1 pc=fc69be",
            "IO read.w $ffff8604 = $0024 pc=fc38b6",
        ]),
        "transients",
    );
    let a = v(&["IO read.b $fffffa01 = $b1 pc=fc69be"]);
    let b = v(&["IO read.b $fffffa01 = $a1 pc=fc69be"]);
    assert!(std::panic::catch_unwind(|| assert_same_sets(&a, &b, "I4")).is_err());
}

/// The ordered sequences see what a set cannot: the order of the IKBD's bytes and a missing FDC command.
#[test]
fn ordered_sequences_see_order_and_count() {
    let a = v(&[
        "IO read.b $fffffc02 = $00 pc=fc07a6",
        "IO read.b $fffffc02 = $01 pc=fc07a6",
    ]);
    let b = v(&[
        "IO read.b $fffffc02 = $01 pc=fc07a6",
        "IO read.b $fffffc02 = $00 pc=fc07a6",
    ]);
    assert_same_sets(&a, &b, "sets miss it");
    assert!(std::panic::catch_unwind(|| assert_same_ordered_sequences(&a, &b, "order")).is_err());
    let c = v(&[
        "IO write.w $ffff8604 = $0080 pc=fc3744",
        "IO write.w $ffff8604 = $0080 pc=fc3744",
    ]);
    let d = v(&["IO write.w $ffff8604 = $0080 pc=fc3744"]);
    assert!(std::panic::catch_unwind(|| assert_same_ordered_sequences(&c, &d, "count")).is_err());
    assert_same_ordered_sequences(&a, &a, "same");
    // A long periodic sequence may be a few entries longer on one side.
    let long: Vec<String> = (0..100)
        .map(|_| "IO write.w $ffff8606 = $0080 pc=fc38a6".to_string())
        .collect();
    assert_same_ordered_sequences(&long, &long[..97], "periodic");
    assert!(
        std::panic::catch_unwind(|| assert_same_ordered_sequences(&long, &long[..90], "too many"))
            .is_err()
    );
}

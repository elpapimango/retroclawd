; pce-vbslow: the vbphase card with the CPU in slow mode (CSL, fact H5): polls the VDC status from reset (vertical blank interrupt enabled in CR, the CPU's interrupts off) and counts polls until the
; vertical blank flag is set ($40-$41), then reads status (clearing it) and counts until it is set again ($42-$43)
    .include "common.inc"
    .org $E000
start:
    preamble
    csl
    stz $40
    stz $41
    stz $42
    stz $43
    st0 #5
    st1 #8
    st2 #0
w1:
    lda $0000
    and #$20
    bne g1
    inc $40
    bne w1
    inc $41
    bra w1
g1:
w2:
    lda $0000
    and #$20
    bne g2
    inc $42
    bne w2
    inc $43
    bra w2
g2:
spin:
    bra spin
    .org $FFF6
    .word $EAEA,$EAEA,$EAEA,$EAEA,start

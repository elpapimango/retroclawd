; P01: the harness smoke test. Writes 16 known bytes at $C000 and loops. Measures nothing; a mismatch between
; Hatari's dump and retroclawd's means the harness (floppy, boot, dump address) differs, not the timing.
; Result: 16 bytes at $C000.
    .org $90000
    move.w  #$2700,sr
    lea     $C000,a0
    move.l  #$50315F53,(a0)+    ; "P1_S"
    move.l  #$4D4F4B45,(a0)+    ; "MOKE"
    move.l  #$00010203,(a0)+
    move.l  #$FEEDFACE,(a0)+
forever:
    bra.s   forever

; P00: harness smoke test. Copies the word $BEEF to the start of work RAM and the long $CAFEF00D to $FF0004, then loops.
; Result: 8 bytes at RAM offset 0 (the oracle reads RETRO_MEMORY_SYSTEM_RAM).
    .org $200
    move.w  #$BEEF,$FF0000
    move.l  #$CAFEF00D,$FF0004
forever:
    bra.s   forever

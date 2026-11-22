; P24: cost of small loops in H-counter units (one unit is about 2.3 cycles), 16 iterations each, display on, H40.
; Result at RAM offset 0: words for: dbf alone, move.w d2,(a0)+dbf (RAM), move.w + nop + dbf, nop-only x16 (baseline 64 cycles).
    .org $200
    lea     $C00004,a1
    lea     $FF0000,a2
    lea     $FF1000,a0
    move.w  #$8004,(a1)
    move.w  #$8144,(a1)
    move.w  #$8C81,(a1)
    move.w  #$1234,d2
    moveq   #0,d5
    bsr.s   sync
    move.w  $C00008,d1
    moveq   #15,d0
la: dbf     d0,la
    move.w  $C00008,d4
    sub.w   d1,d4
    move.w  d4,(a2)+
    bsr.s   sync
    move.w  $C00008,d1
    moveq   #15,d0
lb: move.w  d2,(a0)
    dbf     d0,lb
    move.w  $C00008,d4
    sub.w   d1,d4
    move.w  d4,(a2)+
    bsr.s   sync
    move.w  $C00008,d1
    moveq   #15,d0
lc: move.w  d2,(a0)
    nop
    dbf     d0,lc
    move.w  $C00008,d4
    sub.w   d1,d4
    move.w  d4,(a2)+
    bsr.s   sync
    move.w  $C00008,d1
    moveq   #15,d0
ld: move.w  d2,(a0)
    move.w  d2,(a0)
    dbf     d0,ld
    move.w  $C00008,d4
    sub.w   d1,d4
    move.w  d4,(a2)+
forever:
    bra.s   forever
sync:
    move.w  $C00008,d3
    sub.w   #$3C00,d3
    cmp.w   #$30,d3
    bcc.s   sync
    rts

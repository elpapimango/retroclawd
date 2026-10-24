; P25: the same 8-iteration loop measured two ways, to find why two probes disagreed. Display on, H40.
; Result at RAM offset 0: word A (moveq/loop inline), word B (header move.w d5,d0 / beq / subq), word C (as B, 16 iterations)
    .org $200
    lea     $C00004,a1
    lea     $FF0000,a2
    lea     $FF1000,a0
    move.w  #$8004,(a1)
    move.w  #$8144,(a1)
    move.w  #$8C81,(a1)
    move.w  #$1234,d2
    bsr.s   sync
    move.w  $C00008,d1
    moveq   #7,d0
la: move.w  d2,(a0)
    dbf     d0,la
    move.w  $C00008,d4
    sub.w   d1,d4
    move.w  d4,(a2)+
    moveq   #8,d5
    bsr.s   sync
    move.w  $C00008,d1
    move.w  d5,d0
    beq.s   s1
    subq.w  #1,d0
lb: move.w  d2,(a0)
    dbf     d0,lb
s1: move.w  $C00008,d4
    sub.w   d1,d4
    move.w  d4,(a2)+
    moveq   #16,d5
    bsr.s   sync
    move.w  $C00008,d1
    move.w  d5,d0
    beq.s   s2
    subq.w  #1,d0
lc: move.w  d2,(a0)
    dbf     d0,lc
s2: move.w  $C00008,d4
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

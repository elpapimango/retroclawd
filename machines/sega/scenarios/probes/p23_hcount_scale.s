; P23: the scale of the H counter against 68000 cycles. HV is read at the start of a line, then a run of NOPs (4 cycles each)
; executes, then HV is read again; the low bytes' difference is stored. H40 display on.
; Result at RAM offset 0: words for 32, 24, 16, 8, 0 NOPs.
    .org $200
    lea     $C00004,a1
    lea     $FF0000,a2
    move.w  #$8004,(a1)
    move.w  #$8144,(a1)
    move.w  #$8C81,(a1)
    moveq   #4,d6
    lea     table(pc),a3
again:
    move.w  (a3)+,d5
sync:
    move.w  $C00008,d3
    sub.w   #$3C00,d3
    cmp.w   #$30,d3
    bcc.s   sync
    move.w  $C00008,d1
    jmp     run(pc,d5.w)
run:
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    move.w  $C00008,d4
    sub.w   d1,d4
    move.w  d4,(a2)+
    dbf     d6,again
forever:
    bra.s   forever
table:
    dc.w    0,16,32,48,64

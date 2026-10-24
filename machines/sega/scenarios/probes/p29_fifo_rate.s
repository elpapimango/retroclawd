; P29: the rate at which the data port accepts writes. 200 words are written to VRAM (increment 2) starting at line 20 of the
; display; the number of the line the V counter shows afterwards, minus the starting line, is stored, in H40 and H32, with the
; display on and off, and for CRAM and VSRAM targets (display on).
; Result at RAM offset 0: words: VRAM H40 on, VRAM H40 off, VRAM H32 on, VRAM H32 off, CRAM H40 on, VSRAM H40 on.
    .org $200
    lea     $C00000,a0
    lea     $C00004,a1
    lea     $FF0000,a2
    lea     table(pc),a3
    move.w  #$8004,(a1)
    move.w  #$8F02,(a1)
    moveq   #5,d6
outer:
    move.w  (a3)+,(a1)
    move.w  (a3)+,(a1)
    move.l  (a3)+,(a1)
w1: move.b  $C00008,d3
    cmp.b   #20,d3
    bne.s   w1
    move.w  #199,d0
wr: move.w  d2,(a0)
    dbf     d0,wr
    move.b  $C00008,d4
    sub.b   #20,d4
    move.w  d4,(a2)+
    dbf     d6,outer
forever:
    bra.s   forever
; display register, H40/H32 register, write command
table:
    dc.w    $8144,$8C81
    dc.l    $40000000
    dc.w    $8104,$8C81
    dc.l    $40000000
    dc.w    $8144,$8C00
    dc.l    $40000000
    dc.w    $8104,$8C00
    dc.l    $40000000
    dc.w    $8144,$8C81
    dc.l    $C0000000
    dc.w    $8144,$8C81
    dc.l    $40000010

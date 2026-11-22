; P40: interlace mode 2. Plane A shows 8x16 tiles whose row r has colour r+1, a plane scrolled by an odd number of rows, and a sprite
; two cells tall; the picture of a field is compared with Genesis Plus GX's (single field output).
; Result: the screen (compare_frames-style, scenarios/compare_probe_frames.py).
    .org $200
    lea     $C00000,a0
    lea     $C00004,a1
    move.w  #$8004,(a1)
    move.w  #$8144,(a1)
    move.w  #$8230,(a1)
    move.w  #$8407,(a1)
    move.w  #$8578,(a1)
    move.w  #$8700,(a1)
    move.w  #$8B00,(a1)
    move.w  #$8C87,(a1)
    move.w  #$8D3F,(a1)
    move.w  #$8F02,(a1)
    move.w  #$9001,(a1)
    move.w  #$9100,(a1)
    move.w  #$9200,(a1)
    ; palette: colour n = n * $222
    move.l  #$C0000000,(a1)
    moveq   #0,d0
    moveq   #15,d1
pal:
    move.w  d0,(a0)
    add.w   #$0222,d0
    dbf     d1,pal
    ; VRAM tile 1 at $0020*2 (tile numbers are 64 bytes in this mode: tile 1 = $0040)
    move.l  #$40400000,(a1)
    moveq   #15,d2
t1: move.w  d2,d3
    addq.w  #1,d3
    and.w   #$F,d3
    move.w  d3,d4
    lsl.w   #4,d4
    or.w    d4,d3
    move.w  d3,d4
    lsl.w   #8,d4
    or.w    d4,d3
    move.w  d3,(a0)
    move.w  d3,(a0)
    dbf     d2,t1
    ; tile 2 (at $0080): columns of colours 1..8 on the left, 9..15 on the right, every row alike
    moveq   #15,d2
t2: move.w  #$1234,(a0)
    move.w  #$89AB,(a0)
    dbf     d2,t2
    ; name table plane A at $C000: row 0..31, tile 1 and 2 alternating
    move.l  #$40000003,(a1)
    move.w  #2047,d2
nt: move.w  #$0001,(a0)
    move.w  #$0002,(a0)
    subq.w  #1,d2
    subq.w  #1,d2
    bpl.s   nt
    ; sprite table at $F000: one sprite
    move.l  #$70000003,(a1)
    move.w  #$0190,(a0)
    move.w  #$0100,(a0)
    move.w  #$0002,(a0)
    move.w  #$0090,(a0)
    ; vertical scroll: plane A by 3 rows
    move.l  #$40000010,(a1)
    move.w  #3,(a0)
    move.w  #5,(a0)
forever:
    bra.s   forever

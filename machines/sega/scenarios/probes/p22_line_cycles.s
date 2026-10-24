; P22: 68000 cycles per scanline, as the VDP's V counter sees them. A loop of known cost (addq.l 8 + move.b 16 + cmp.b 8 + bne 10
; = 42 cycles) counts how many iterations fit between V = 60 and V = 80 (20 lines), once with the display off and once on,
; with 256 lines' worth ... only the count is stored.
; Result at RAM offset 0: word iterations (display off), word iterations (display on).
    .org $200
    lea     $C00004,a1
    lea     $FF0000,a2
    move.w  #$8004,(a1)
    move.w  #$8F02,(a1)
    move.w  #$8C81,(a1)
    moveq   #1,d6
    move.w  #$8104,(a1)
again:
w1: move.b  $C00008,d3
    cmp.b   #60,d3
    bne.s   w1
    moveq   #0,d7
w2: addq.l  #1,d7
    move.b  $C00008,d3
    cmp.b   #80,d3
    bne.s   w2
    move.w  d7,(a2)+
    move.w  #$8144,(a1)
    dbf     d6,again
forever:
    bra.s   forever

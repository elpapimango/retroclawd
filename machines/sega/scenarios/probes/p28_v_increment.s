; P28: where in the line the V counter changes. The HV counter is read in a tight loop (16 cycles per read) from line 59 on, and
; the words are stored: three lines' worth. H40, display on.
; Result at RAM offset 0: 96 words.
    .org $200
    lea     $C00004,a1
    lea     $FF0000,a2
    move.w  #$8004,(a1)
    move.w  #$8144,(a1)
    move.w  #$8C81,(a1)
w1: move.b  $C00008,d3
    cmp.b   #59,d3
    bne.s   w1
    moveq   #95,d6
rd: move.w  $C00008,(a2)+
    dbf     d6,rd
forever:
    bra.s   forever

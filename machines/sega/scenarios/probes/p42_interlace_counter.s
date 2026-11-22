; P42: the V counter and the field flag in the interlaced modes. At the same point of four successive frames the V counter byte and
; the status register's bit 4 are read, with interlace off, in mode 1 and in mode 2 (the target V value is the one line 61 shows).
; Result at RAM offset 0: for each of off, mode 1, mode 2: four pairs (V counter byte, status byte & $10).
    .org $200
    lea     $C00004,a1
    lea     $FF0000,a2
    lea     table(pc),a3
    move.w  #$8004,(a1)
    move.w  #$8144,(a1)
    moveq   #2,d6
outer:
    move.w  (a3)+,(a1)
    move.w  (a3)+,d5
    moveq   #3,d7
frames:
wx: move.b  $C00008,d3
    cmp.b   d5,d3
    beq.s   wx
w2: move.b  $C00008,d3
    cmp.b   d5,d3
    bne.s   w2
    move.b  $C00008,(a2)+
    move.b  $C00005,d4
    and.b   #$10,d4
    move.b  d4,(a2)+
    dbf     d7,frames
    dbf     d6,outer
forever:
    bra.s   forever
table:
    dc.w    $8C81,61,$8C83,60,$8C87,122

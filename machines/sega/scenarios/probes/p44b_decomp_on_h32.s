; P44: the speed of the loop Aladdin's decompressor spends its time in: two bytes copied with post-increment, the word just written
; copied to the VDP data port, DBF. 1500 iterations from line 10 of the frame with the display on (so the FIFO does not hold the
; 68000), source ROM, destination work RAM. Result at RAM offset 0: the HV counter before and after (two words).
    .org $200
    lea     $C00004,a1
    lea     $FF0000,a2
    move.w  #$8004,(a1)
    move.w  #$8144,(a1)
    move.w  #$8C00,(a1)
    move.w  #$8F02,(a1)
    move.l  #$40000000,(a1)
w1: move.w  $C00008,d3
    sub.w   #$0A00,d3
    cmp.w   #$30,d3
    bcc.s   w1
    move.w  $C00008,d5
    lea     $0,a0
    lea     $FF1000,a3
    move.w  #1499,d1
lp: move.b  (a0)+,(a3)+
    move.b  (a0)+,(a3)+
    move.w  -$2(a3),$C00000
    dbf     d1,lp
    move.w  $C00008,d6
    move.w  d5,(a2)+
    move.w  d6,(a2)+
forever:
    bra.s   forever

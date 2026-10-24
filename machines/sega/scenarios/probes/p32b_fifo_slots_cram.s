; P32: the times at which the VDP accepts data-port writes. After line 20 starts, words are written to VRAM (or CRAM) one after
; another and the HV counter is read after each; once the FIFO is full every write ends when the VDP frees an entry, so the
; counter values after the fifth write on show the VDP's access slots. Display on, H40.
; Result at RAM offset 0: 120 words, the HV counter after each of 120 writes.
; mkrom: 
    .org $200
    lea     $C00000,a0
    lea     $C00004,a1
    lea     $FF0000,a2
    move.w  #$8004,(a1)
    move.w  #$8144,(a1)
    move.w  #$8C81,(a1)
    move.w  #$8F02,(a1)
    move.l  #$C0000000,(a1)
    move.w  #$1234,d2
w1: move.w  $C00008,d3
    sub.w   #$1400,d3
    cmp.w   #$20,d3
    bcc.s   w1
    moveq   #119,d6
wr: move.w  d2,(a0)
    move.w  $C00008,(a2)+
    dbf     d6,wr
forever:
    bra.s   forever

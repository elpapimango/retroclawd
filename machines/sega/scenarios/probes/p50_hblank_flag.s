; P50: where in a line the HBLANK status bit (bit 2) changes. At line 100 the status register is read in a tight loop;
; each time bit 2 differs from the last read, the HV counter is read at once and stored. Result at RAM offset 0: eight words
; (the HV counter after each of the first eight changes), H40 then (second run in the same ROM) H32 at offset 16.
    .org $200
    lea     $C00004,a1
    lea     $C00008,a4
    lea     $FF0000,a2
    move.w  #$8004,(a1)
    move.w  #$8F02,(a1)
    move.w  #$8154,(a1)
    move.w  #$8C81,(a1)
    moveq   #1,d6
again:
w0: move.b  (a4),d3
    cmp.b   #100,d3
    bne.s   w0
    moveq   #7,d7
    move.w  (a1),d0
    and.w   #4,d0
loop:
    move.w  (a1),d1
    and.w   #4,d1
    cmp.w   d0,d1
    beq.s   loop
    move.w  d1,d0
    move.w  (a4),(a2)+
    dbf     d7,loop
    move.w  #$8C00,(a1)
    dbf     d6,again
forever:
    bra.s   forever

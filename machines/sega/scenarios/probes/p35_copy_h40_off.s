; P34 (p35_copy_h40_off): the time a VDP DMA of 1000 units takes: copy, H40, display off, starting at line 20.
; The word stored is how many lines the V counter advanced until the 68000 was free again (the busy flag cleared for a fill or a
; copy). Result at RAM offset 0: one word.
    .org $200
    lea     $C00000,a0
    lea     $C00004,a1
    lea     $FF0000,a2
    move.w  #$8004,(a1)
    move.w  #$8F02,(a1)
    move.w  #$8114,(a1)
    move.w  #$8C81,(a1)
    move.w  #$93E8,(a1)
    move.w  #$9403,(a1)
    move.w  #$9500,(a1)
    move.w  #$9600,(a1)
    move.w  #$97C0,(a1)
w0: move.b  $C00008,d3
    cmp.b   #200,d3
    bne.s   w0
w1: move.b  $C00008,d3
    cmp.b   #20,d3
    bne.s   w1
    move.w  #$0000,(a1)
    move.w  #$00C0,(a1)
fl: btst    #1,$C00005
    bne.s   fl
    move.b  $C00008,d4
    sub.b   #20,d4
    and.w   #$FF,d4
    move.w  d4,(a2)+
forever:
    bra.s   forever

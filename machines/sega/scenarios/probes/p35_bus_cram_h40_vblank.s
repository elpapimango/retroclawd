; P34 (p35_bus_cram_h40_vblank): the time a VDP DMA of 500 units takes: bus cram, H40, display on, starting at line 230.
; The word stored is how many lines the V counter advanced until the 68000 was free again (the busy flag cleared for a fill or a
; copy). Result at RAM offset 0: one word.
    .org $200
    lea     $C00000,a0
    lea     $C00004,a1
    lea     $FF0000,a2
    move.w  #$8004,(a1)
    move.w  #$8F02,(a1)
    move.w  #$8154,(a1)
    move.w  #$8C81,(a1)
    move.w  #$93F4,(a1)
    move.w  #$9401,(a1)
    move.w  #$9500,(a1)
    move.w  #$9600,(a1)
    move.w  #$9700,(a1)
w1: move.b  $C00008,d3
    cmp.b   #230,d3
    bne.s   w1
    move.w  #$C000,(a1)
    move.w  #$0080,(a1)
    move.b  $C00008,d4
    sub.b   #230,d4
    and.w   #$FF,d4
    move.w  d4,(a2)+
forever:
    bra.s   forever

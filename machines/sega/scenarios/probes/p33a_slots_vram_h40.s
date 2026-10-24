; P33 (p33a_slots_vram_h40): when the VDP accepts the n-th data-port write after the horizontal interrupt. The 68000 sleeps in STOP;
; every line the handler jumps into a run of 44 writes to the data port, skipping all but n of them (one more each line), and then
; reads the HV counter. Display on, H40. Result at RAM offset 0: 44 pairs of words (HV counter at the start, HV counter at the end; n = 1 .. 44).
; mkrom: --vector 28=0x204
    .org $200
    bra.w   main
hint:
    tst.w   d5
    bmi.w   out
    move.w  $C00008,(a2)+
    jmp     run(pc,d5.w)
run:
    move.w  d2,(a0)
    move.w  d2,(a0)
    move.w  d2,(a0)
    move.w  d2,(a0)
    move.w  d2,(a0)
    move.w  d2,(a0)
    move.w  d2,(a0)
    move.w  d2,(a0)
    move.w  d2,(a0)
    move.w  d2,(a0)
    move.w  d2,(a0)
    move.w  d2,(a0)
    move.w  d2,(a0)
    move.w  d2,(a0)
    move.w  d2,(a0)
    move.w  d2,(a0)
    move.w  d2,(a0)
    move.w  d2,(a0)
    move.w  d2,(a0)
    move.w  d2,(a0)
    move.w  d2,(a0)
    move.w  d2,(a0)
    move.w  d2,(a0)
    move.w  d2,(a0)
    move.w  d2,(a0)
    move.w  d2,(a0)
    move.w  d2,(a0)
    move.w  d2,(a0)
    move.w  d2,(a0)
    move.w  d2,(a0)
    move.w  d2,(a0)
    move.w  d2,(a0)
    move.w  d2,(a0)
    move.w  d2,(a0)
    move.w  d2,(a0)
    move.w  d2,(a0)
    move.w  d2,(a0)
    move.w  d2,(a0)
    move.w  d2,(a0)
    move.w  d2,(a0)
    move.w  d2,(a0)
    move.w  d2,(a0)
    move.w  d2,(a0)
    move.w  d2,(a0)
    move.w  $C00008,d4
    move.w  d4,(a2)+
    subq.w  #2,d5
out:
    rte
main:
    lea     $C00000,a0
    lea     $C00004,a1
    lea     $FF0000,a2
    move.w  #$8004,(a1)
    move.w  #$8014,(a1)
    move.w  #$8144,(a1)
    move.w  #$8A0A,(a1)
    move.w  #$8C81,(a1)
    move.w  #$8F02,(a1)
    move.l  #$40000000,(a1)
    move.w  #$1234,d2
    move.w  #86,d5
sleep:
    stop    #$2000
    bra.s   sleep

; P27: the H counter against time, measured from the horizontal interrupt. The 68000 sleeps in STOP; every line the
; interrupt handler jumps into a sled of NOPs (4 cycles each), skipping all but k of them, and stores the H counter. k = 0..119,
; one more NOP each line. The sample for k is the H counter that many cycles after the interrupt was taken.
; Result at RAM offset 0: 120 bytes (H counter low byte).
; mkrom: --vector 28=0x204
    .org $200
    bra.w   main
hint:
    tst.w   d5
    bmi.w   out
    jmp     run(pc,d5.w)
run:
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    move.w  $C00008,d4
    move.b  d4,(a2)+
    subq.w  #2,d5
out:
    rte
main:
    lea     $C00004,a1
    lea     $FF0000,a2
    move.w  #$8004,(a1)
    move.w  #$8014,(a1)
    move.w  #$8144,(a1)
    move.w  #$8A00,(a1)
    move.w  #$8C81,(a1)
    move.w  #240,d5
sleep:
    stop    #$2000
    bra.s   sleep
